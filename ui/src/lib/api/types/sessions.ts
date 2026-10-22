// Contract mirror: sessions. Part of lib/api/types (see ../types.ts, the barrel).
// Mirrors docs/contracts/api.md + ws.md — change them together.

import type {
  Id, SessionKind, SessionStatus,
} from './core';

export interface Session {
  id: Id;
  workspace_id: Id;
  kind: SessionKind;
  provider: string;
  title: string;
  status: SessionStatus;
  cwd: string;
  provider_session_id: string | null;
  connection_id: Id | null;
  created_by: Id;
  created_at: string;
  last_active_at: string;
  archived: boolean;
  meta: Record<string, unknown>;
  /** Transient (list/get only): a PTY is live for this session right now.
   *  Absent on payloads that return the bare domain row (archive, events). */
  live?: boolean;
  /** Transient (list/get only): attached `/ws/term` viewer count. */
  viewers?: number;
  /** Transient (list/get only): the live PTY runs in a PTY holder and
   *  session persistence is on — it survives a daemon restart. */
  held?: boolean;
}

/** Query for `GET /workspaces/{id}/sessions` (#17) and the cross-workspace
 *  `GET /sessions` (#17b; there `archived` defaults to `false`). Every filter
 *  runs in SQL. `limit` (1–1000) keeps the newest N rows (still oldest-first);
 *  `before` is an RFC 3339 `created_at` cursor for paging back. */
export interface SessionListQuery {
  archived?: boolean;
  kind?: SessionKind;
  /** Exact `meta.source`; `'none'` = no string source (foreground sessions). */
  source?: string;
  status?: SessionStatus;
  limit?: number;
  before?: string;
}

/** Body for `POST /sessions/bulk`. */
export interface BulkSessionsReq {
  action: 'archive' | 'delete' | 'kill';
  ids: Id[];
}

/** Per-id outcome of `POST /sessions/bulk`. */
export interface BulkSessionResult {
  id: Id;
  ok: boolean;
  error?: string;
}

// ---------------------------------------------------------------------------
// Sessions
// ---------------------------------------------------------------------------

export interface CreateSessionReq {
  kind: SessionKind;
  provider?: string | null;
  title?: string | null;
  cwd?: string | null;
  connection_id?: Id | null;
  meta?: Record<string, unknown> | null;
  /** Model to pin for THIS session only (copied into `meta.model`, expanded via
   *  the provider's model-flag template). Absent/empty = provider default. */
  model?: string | null;
}

export interface UpdateSessionReq {
  title?: string | null;
  /** Shallow-merged into the session's existing meta (e.g. `extra_dirs`). */
  meta?: Record<string, unknown> | null;
}

/** POST /sessions/{id}/input — write text into a session's PTY server-side. */
export interface SendInputReq {
  /** Text to write into the PTY. */
  text: string;
  /** Append a newline so the agent runs it immediately. `null`/`true` submits;
   *  `false` sends verbatim so the user can inspect/edit before pressing Enter. */
  submit?: boolean | null;
}

/** POST /workspaces/{id}/sessions/open — open an agent session for a delegating
 *  lead and queue its opening prompt once the TUI is up. */
export interface OpenAgentSessionReq {
  provider: string;
  title?: string | null;
  cwd?: string | null;
  model?: string | null;
  /** Submitted as the session's first user message (async; see `prompt_dispatch`). */
  prompt?: string | null;
  meta?: Record<string, unknown> | null;
}

export interface OpenAgentSessionResp {
  session: Session;
  /** `"queued"` when an opening prompt is being delivered, `"none"` otherwise. */
  prompt_dispatch: 'queued' | 'none';
}

/** POST /sessions/{id}/message — one message to ONE live agent session (typed + Enter). */
export interface SessionMessageReq {
  text: string;
}

export interface SessionMessageResp {
  session_id: Id;
  delivered: boolean;
}

/** GET /sessions/{id}/wait — the session once an awaited status was reached, or at the deadline. */
export interface WaitSessionResp {
  session: Session;
  reached: boolean;
}

// ---------------------------------------------------------------------------
// Per-session tokens + cost (review A5) — GET /sessions/{id}/usage and
// GET /workspaces/{wid}/sessions/usage
// ---------------------------------------------------------------------------

/** One session's usage sums (otto-usage `SessionTotals`). */
export interface SessionTotals {
  session_id: Id;
  workspace_id: Id;
  provider: string;
  events: number;
  input_tokens: number;
  output_tokens: number;
  cache_read_tokens: number;
  cache_write_tokens: number;
  total_tokens: number;
  cost_usd: number;
}

/** `GET /workspaces/{wid}/sessions/usage?days=` — sessions without usage are absent. */
export interface WorkspaceSessionsUsage {
  /** False when the usage engine is unavailable: hide tokens/cost UI. */
  available: boolean;
  days: number;
  sessions: SessionTotals[];
}

// ── Session list query (perf: Agents page, F1/F3) ────────────────────────────
/** Query of `GET /workspaces/{id}/sessions` (#17) and `GET /sessions` (#17b).
 *  Every field narrows in SQL. Comma lists are sent comma-joined. */
export interface SessionListQuery {
  archived?: boolean;
  kind?: 'agent' | 'connection';
  /** Exact `meta.source`; `'none'` = no string source. */
  source?: string;
  status?: SessionStatus;
  /** Newest N matching rows (1–1000), still returned oldest-first. */
  limit?: number;
  /** RFC 3339 cursor: rows created strictly before it. */
  before?: string;
  /** `true` = sidebar rows (connections + foreground agents + `with_sources`);
   *  `false` = background agents only. */
  foreground?: boolean;
  /** Background sources to keep with `foreground=true` (≤ 64), e.g. `channel`. */
  with_sources?: string[];
  /** Fetch-by-id (≤ 64). On #17b it also lifts the `archived=false` default. */
  ids?: Id[];
}
