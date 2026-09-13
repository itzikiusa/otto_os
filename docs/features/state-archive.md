# Complete saved-state archives

Settings → Backup & Restore offers a version 2 JSON archive of Otto's saved records and owned document assets. The older settings export and settings restore remain separate, settings-only operations. A root user can download the archive, inspect another archive, review conflicts, and restore additively. Git sync uses the same schema validation and credential filtering, with a smaller portable configuration/document selection.

## Contents

The exporter enumerates the installed SQLite schema within one read transaction; each included table is recorded by name with all its saved rows and columns. It includes workspace metadata, sessions and task/trail history, repository/account metadata, reviews and proof records, API collections/requests/environments/automations/history/reports, connections and saved database/broker/cloud/Kubernetes records, Product stories and histories, Canvas scenes, Vault registrations, memories, work-graph records, workflow definitions/versions/runs, schedules, personal agents, swarms, plugin metadata, and audits. Binary SQLite values use `{ "$base64": "..." }`.

Owned files come from `library`, `canvas`, `product`, `snips`, `transcripts`, `workflow-context`, `scheduled`, `personal`, and `insights` under Otto's data directory, plus every registered Vault root. The archive records each root by a trusted label, each relative file path, base64 bytes, and SHA-256. A missing registered Vault root, symlink/special file, changing file, or size-limit violation fails the export instead of returning a silently incomplete archive. Ordinary document bytes are preserved, including Markdown, Canvas source, mockups and attachments.

The manifest explicitly lists exclusions:

- Keychain contents, authentication sessions, password hashes, secret references, configured credential fields, secret environment/header entries, and credentials embedded in structured transport URLs/configuration.
- Active permission bindings and allowlists: `workspace_members`, `access_group_members`, `user_feature_grants`, `plugin_feature_grants`, `resource_access_policies`, `resource_access_policy_versions`, `mcp_allowlist`, and `mcp_policies`. Re-establish access deliberately after import.
- Derived Vault note/link/tag/file indexes and FTS tables, provider models, provider transcript indexes, workflow caches, plugin migration bookkeeping, SQLite internal tables, and migration bookkeeping.
- Live processes, sockets, PTY state, external repository working trees, workflow/agent worktrees, database-engine data, and external provider transcript directories. Their saved references/history remain available where applicable.
- Git metadata, `.otto-sync` output, `.env`/`.env.*`, private-key/certificate containers, and known credential-file names. Every encountered excluded credential file is listed; generated `.otto-sync` has a stable global exclusion.

Archives contain private free-form documents and histories. A user-written prompt or note may contain sensitive content; the archive is not a public-sharing sanitizer. Configured credentials are removed and per-record reconnect notices are provided. No Keychain read is performed by this exporter. The separate Connections export feature controls any explicitly requested password export.

## Restore behavior

An archive must match the installed database migration version. Restore validates its table/column names, explicit primary keys, root ownership, relative paths, file sizes/encoding/hashes, SQLite CHECK/NOT NULL constraints, and deferred foreign keys before publishing files. The preview simulates inserts in a rolled-back transaction and reports counts, conflicts, exclusions, reconnect notices, and validation failures.

`abort` refuses any existing primary/unique key or destination file. `skip_existing` keeps existing records and files. Existing Canvas, Product attachment/story, session-transcript, and Vault owners also retain their associated assets. There is no replace or delete mode. Existing users with the same username are mapped to their local IDs; their credentials and permissions are retained. Newly imported users are disabled and have no password or root role. Imported services/schedules are disabled, active runs are marked interrupted/error, and approvals expire; restore does not launch agents, sessions, workflows, or remote operations.

Vault files are relocated under `restored/<archive-id>/vaults/<vault-id>` and the saved Vault root is rewritten; its derived index is rebuilt. Canvas and Product assets are published in their normal owned layouts so existing readers can access them. Absolute references to included managed files are rewritten to this profile's data directory; external paths remain references and may need remapping. No external repository is created, replaced, or checked out.

The preview token binds archive bytes, chosen conflict policy, affected insertion/conflict outcomes, user mapping, and destination existence. Changes that affect the plan require a new preview. Unrelated background/audit rows do not invalidate it. Restore stages files before publication, uses exclusive creation with descriptor-relative symlink refusal, then commits all database inserts together. On ordinary failures it removes only unchanged files it created; pre-existing files are retained. An approved restore runs independently of an HTTP client disconnect. Empty created directories may remain after a failure. A process/OS crash is not a cross-filesystem/SQLite atomic snapshot and may leave unpublished metadata or orphan new files; inspect and preview again after recovery.

## Limits and API

The serialized archive limit is 256 MiB, including base64 overhead; each decoded file is limited to 64 MiB. The HTTP request allows 1 MiB extra envelope space. Oversize export/import is rejected. Database records share a read snapshot; filesystem assets are read individually with change detection, so save edits and pause writers before exporting when cross-file consistency matters. Restored provider/plugin settings may require a daemon restart and deliberate re-enabling.

Root-only endpoints:

- `GET /api/v1/state/archive`
- `POST /api/v1/state/archive/preview` with `{ archive, conflicts }`
- `POST /api/v1/state/archive/restore` with `{ archive, conflicts, preview_token, confirm: true }`

The authoritative DTOs are in [the API contract](../contracts/api.md). Git portable snapshots include saved configurations/documents and only `library`, `canvas`, `product`, and registered Vault assets; runtime history directories are excluded. Git does not recreate authentication or runtime execution state.
