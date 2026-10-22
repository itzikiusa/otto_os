// Contract mirror: connections. Part of lib/api/types (see ../types.ts, the barrel).
// Mirrors docs/contracts/api.md + ws.md — change them together.

import type {
  Id,
} from './core';

export type ConnectionKind =
  | 'ssh'
  | 'mysql'
  | 'postgres'
  | 'redis'
  | 'mongodb'
  | 'clickhouse'
  | 'custom';

/** Deployment environment. `prod` is write-guarded in the DB Explorer. */
export type Environment = 'dev' | 'staging' | 'prod';

export interface Connection {
  id: Id;
  workspace_id: Id | null;
  name: string;
  kind: ConnectionKind;
  params: Record<string, unknown>;
  secret_ref: string | null;
  first_command: string | null;
  section_id: Id | null;
  /** Deployment environment (defaults to 'dev'). */
  environment: Environment;
  /** When true, writes/DDL are refused without an explicit confirmation. */
  read_only: boolean;
  created_by: Id;
  created_at: string;
  /** ISO timestamp of the last open; absent means never opened. */
  last_opened_at?: string | null;
  /** When true the connection floats to the top of the list regardless of recency. */
  pinned?: boolean;
}

export interface ConnectionSection {
  id: Id;
  workspace_id: Id;
  parent_id: Id | null;
  name: string;
  position: number;
  /** Which tree this section belongs to: 'connections' (Connections page) or 'db' (DB Explorer). */
  scope: string;
  created_by: Id;
  created_at: string;
}

// ---------------------------------------------------------------------------
// Connections
// ---------------------------------------------------------------------------

export interface UpsertConnectionReq {
  name: string;
  kind: ConnectionKind;
  params: Record<string, unknown>;
  secret?: string | null;
  first_command?: string | null;
  section_id?: Id | null;
  /** Deployment environment. On create, omit for 'dev'; on PATCH, omitting it
   *  KEEPS the stored value (never silently downgrades a 'prod' connection). */
  environment?: Environment;
  /** Lock the profile against writes/DDL. On create, omit for false; on PATCH,
   *  omitting it KEEPS the stored value (never silently un-locks read-only). */
  read_only?: boolean;
}

export interface TestConnectionResp {
  ok: boolean;
  latency_ms: number | null;
  message: string;
  warn_argv: boolean;
  /** Set when the connection's SSH private key file has insecure (group/other-
   *  readable) permissions; carries the full message incl. the `chmod 600 <path>`
   *  fix. Independent of `ok`. */
  warn_key_perms?: string | null;
  /** What to change when `ok` is false and the failure is a recognised one
   *  (auth rejected, host key changed, DNS, refused, timed out, …). */
  hint?: string | null;
}

/** `POST /connections/unsaved/test` — SSH test-before-save (root only); probes
 *  the form's current values, persists nothing. Database kinds use
 *  `/connections/unsaved/db/test`. Responds with `TestConnectionResp`. */
export interface TestUnsavedConnectionReq {
  workspace_id: string;
  kind: 'ssh';
  params: Record<string, unknown>;
}

// ---------------------------------------------------------------------------
// Import connections from other DB tools
// ---------------------------------------------------------------------------

/** A DB tool Otto can import saved connections from. The daemon runs locally
 *  and reads each tool's config from its default macOS location — the user
 *  picks a tool, never a file. */
export type ImportSource = 'mysql_workbench' | 'dbeaver' | 'datagrip' | 'nosqlbooster';

/** One tool's availability (GET …/connections/import/sources). */
export interface SourceStatus {
  source: ImportSource;
  label: string;
  /** True when a config file was found at the default location. */
  present: boolean;
  /** The resolved config path (first match), if any. */
  path?: string | null;
  /** How many connections were parsed out, when present (may be 0). */
  count?: number | null;
}

/** A single connection parsed from a tool's config. When `supported`, `params`
 *  is the ready-to-create Otto shape; otherwise `kind` is null and `note`
 *  explains why the engine was skipped. */
export interface ParsedConnection {
  source: ImportSource;
  name: string;
  /** null for an engine Otto doesn't support (still listed so the user sees why). */
  kind?: ConnectionKind | null;
  params: Record<string, unknown>;
  supported: boolean;
  /** True when the source had a username but no recoverable password — the user
   *  must add it after import (MongoDB uses a `{secret}` placeholder). */
  needs_password: boolean;
  note?: string | null;
}

/** Result of scanning a single tool (POST …/connections/import/scan). */
export interface ImportScanResult {
  source: ImportSource;
  path?: string | null;
  connections: ParsedConnection[];
  warnings: string[];
}

/** One connection the user chose to create. */
export interface ImportCreateItem {
  action?: 'create' | 'update' | 'skip';
  target_id?: string;
  name: string;
  kind: ConnectionKind;
  params: Record<string, unknown>;
  environment?: Environment;
  read_only?: boolean;
}

/** POST …/connections/import/create body. */
export interface ImportCreateReq {
  connections: ImportCreateItem[];
  section_id?: Id | null;
}

/** Result of an import create batch (best-effort — partial success is fine). */
export interface ImportCreateResult {
  updated: Connection[];
  skipped: string[];
  created: Connection[];
  failed: { name: string; error: string }[];
}

// ---------------------------------------------------------------------------
// SFTP file browser (over an SSH connection's existing auth)
// ---------------------------------------------------------------------------

/** One entry in a remote directory listing (GET /connections/{id}/sftp/list). */
export interface SftpEntry {
  name: string;
  kind: 'dir' | 'file' | 'symlink' | 'other';
  size: number;
  /** Raw date/time field from the listing (e.g. "Jun 20 12:00"), if present. */
  mtime: string | null;
  /** The 10-char permission string (e.g. "drwxr-xr-x"). */
  perms: string;
  /** For symlinks, the link target; null otherwise. */
  symlink_target: string | null;
}

export interface SftpListResp {
  /** Absolute remote path that was listed (resolved from pwd when omitted). */
  path: string;
  entries: SftpEntry[];
  /** True when the directory held more than the daemon's 20k-entry cap;
   *  `entries` is then the first 20k. Omitted when false. */
  truncated?: boolean;
}

/** `POST /api/v1/connections/{id}/sftp/download`. */
export interface SftpDownloadReq {
  remote_path: string;
  /** Local destination (file path or dir). Leading `~` expands to daemon home. */
  local_path: string;
}

export interface SftpDownloadResp {
  local_path: string;
  bytes: number;
}

/** `POST /api/v1/connections/{id}/sftp/upload`. */
export interface SftpTransferReq {
  direction: 'download' | 'upload'; local_path: string; remote_path: string; timeout_secs?: number;
}
export interface SftpTransfer {
  id: string; direction: 'download' | 'upload'; local_path: string; remote_path: string;
  status: 'running' | 'finalizing' | 'completed' | 'cancelled' | 'failed' | 'timed_out' | 'outcome_unknown';
  bytes: number; total_bytes: number | null; elapsed_secs: number; error: string | null;
}

export interface SftpUploadReq {
  /** Local source (leading `~` expands to daemon home). */
  local_path: string;
  remote_path: string;
}

/** `POST /api/v1/connections/{id}/sftp/mkdir`. */
export interface SftpMkdirReq {
  path: string;
}

/** `POST /api/v1/connections/{id}/sftp/remove` — `dir:true` ⇒ rmdir, else rm. */
export interface SftpRemoveReq {
  path: string;
  dir?: boolean;
}

/** `POST /api/v1/connections/{id}/sftp/rename`. */
export interface SftpRenameReq {
  from: string;
  to: string;
}

/** `GET /api/v1/connections/{id}/sftp/read?path=` — text view of a small file. */
export interface SftpReadResp {
  text: string;
  /** True when the file exceeded the read cap (content is the capped prefix). */
  truncated: boolean;
}
