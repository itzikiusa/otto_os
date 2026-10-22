// Contract mirror: workbench. Part of lib/api/types (see ../types.ts, the barrel).
// Mirrors docs/contracts/api.md + ws.md — change them together.

import type {
  Id,
} from './core';

// --- Workbench (scratch files with full history) -------------------------------
// Mirrors docs/contracts/api.md "Workbench" + crates/otto-state/src/workbench.rs.
// Docs are per-user inside a workspace; every save lands in an append-only
// revision history that is only removed by an explicit permanent delete.

/** Revision kinds: `create` (first content), `auto` (an autosave burst,
 *  coalesced to one revision per ~60 s), `checkpoint` (⌘S save-now — always a
 *  new revision), `restore` (content restored from an older revision),
 *  `rename`-free: metadata changes never create revisions. */
export type WorkbenchRevisionKind = 'create' | 'auto' | 'checkpoint' | 'restore';

/** Where the owner sees a doc: `workspace` = only in the workspace it lives in
 *  (`workspace_id`); `global` = in every workspace (still owner-only). */
export type WorkbenchScope = 'workspace' | 'global';

/** List row / metadata (no content). */
export interface WorkbenchDoc {
  id: Id;
  /** The workspace the doc lives in (for a `global` doc: where it was created). */
  workspace_id: Id;
  owner_id: Id;
  scope: WorkbenchScope;
  name: string;
  /** Language id (cm-langs extension style: `json`, `md`, `sql`, `html`,
   *  `mermaid`, `d2`, `csv`, `yaml`, `toml`, `xml`, `sh`, `py`, `js`, `ts`,
   *  `txt`, `image`, …). `auto` = let the UI detect it from name + content. */
  language: string;
  pinned: boolean;
  /** Optional folder path (`/`-separated, no leading slash); `''` = root. */
  folder: string;
  tags: string[];
  /** UTF-8 byte length of the current content. */
  size: number;
  /** Seq of the newest revision (1-based, monotonically increasing). */
  rev: number;
  /** sha256 hex of the current content. */
  content_hash: string;
  created_at: string;
  updated_at: string;
  /** Set while the doc is in the trash (soft-deleted); null otherwise. */
  deleted_at: string | null;
}

/** `GET /workbench/docs/{id}` — metadata + current content. For an `image`
 *  doc `content` is the asset id (see `WorkbenchAsset`). */
export interface WorkbenchDocFull extends WorkbenchDoc {
  content: string;
}

export interface WorkbenchCreateReq {
  name: string;
  language?: string;
  content?: string;
  folder?: string;
  tags?: string[];
  pinned?: boolean;
  /** Default `workspace`. */
  scope?: WorkbenchScope;
}

/** `PATCH /workbench/docs/{id}` — every field optional. `content` triggers a
 *  revision (coalesced unless `checkpoint`); metadata never does. */
export interface WorkbenchUpdateReq {
  content?: string;
  name?: string;
  language?: string;
  pinned?: boolean;
  folder?: string;
  tags?: string[];
  /** `global` shows it in every workspace; `workspace` scopes it to the
   *  workspace of this request (moving it here if it lived elsewhere). */
  scope?: WorkbenchScope;
  /** Force a fresh revision (⌘S "save now") instead of coalescing. */
  checkpoint?: boolean;
  /** Opaque per-window id echoed in the WS event so a window can ignore its
   *  own writes. */
  client_id?: string;
  /** Precondition for a `content` write: the `content_hash` the buffer was
   *  based on. Mismatch → 409 (another window saved). Omit to overwrite. */
  if_hash?: string;
}

export interface WorkbenchRevision {
  seq: number;
  kind: WorkbenchRevisionKind;
  /** sha256 hex of this revision's content (content-addressed blob). */
  content_hash: string;
  size: number;
  /** When the revision (burst) started. */
  created_at: string;
  /** Last write folded into this revision (== created_at unless coalesced). */
  updated_at: string;
  /** Number of saves coalesced into this revision (1 = a single save). */
  saves: number;
  /** For `restore`: the seq whose content was restored. */
  restored_from?: number | null;
}

export interface WorkbenchRevisionDetail extends WorkbenchRevision {
  doc_id: Id;
  content: string;
}

export type WorkbenchDiffOp = 'eq' | 'add' | 'del';

export interface WorkbenchDiffLine {
  op: WorkbenchDiffOp;
  text: string;
  /** 1-based line numbers in the old / new text (absent on the other side). */
  old_line?: number | null;
  new_line?: number | null;
}

/** `GET /workbench/docs/{id}/diff?from=<seq>&to=<seq|current>` (line diff).
 *  `to` defaults to the current content. */
export interface WorkbenchDiff {
  doc_id: Id;
  from: number;
  /** A seq, or `null` when diffed against the current content. */
  to: number | null;
  added: number;
  removed: number;
  lines: WorkbenchDiffLine[];
}

/** An uploaded binary (image) stored with the workbench: `POST
 *  /workspaces/{ws}/workbench/assets` (raw body, image/* content type). */
export interface WorkbenchAsset {
  id: Id;
  mime: string;
  size: number;
  sha256: string;
  created_at: string;
}

/** WS: a workbench doc changed (owner-only delivery). Invalidation cue: refetch
 *  the list and, if open and not self-originated, the doc. */
export interface WorkbenchDocChangedEvent {
  type: 'workbench_doc_changed';
  workspace_id: Id;
  user_id: Id;
  doc_id: Id;
  /** A `global` doc shows in every workspace, so windows on other
   *  workspaces than `workspace_id` refetch too. */
  scope: WorkbenchScope;
  action: 'created' | 'updated' | 'trashed' | 'restored' | 'deleted';
  rev: number;
  /** Content hash after the change (a coalesced autosave keeps `rev`). */
  content_hash: string;
  updated_at: string;
  client_id?: string | null;
}
