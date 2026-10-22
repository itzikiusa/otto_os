// Contract mirror: activity. Part of lib/api/types (see ../types.ts, the barrel).
// Mirrors docs/contracts/api.md + ws.md — change them together.

import type {
  Id,
} from './core';

// ---------------------------------------------------------------------------
// Agent activity (live trail + task tracker) — mirrors otto_core::domain
// ---------------------------------------------------------------------------

export type TrailSource = 'user' | 'agent' | 'otto';
export type TrailKind =
  | 'session'
  | 'prompt'
  | 'skill'
  | 'command'
  | 'tool'
  | 'file'
  | 'web'
  | 'task'
  | 'note'
  | 'other';

/** One entry in a session's live activity trail. */
export type TrailLevel = 'info' | 'warn' | 'error';

export interface TrailEvent {
  id: Id;
  session_id: Id;
  workspace_id: Id;
  ts: string;
  source: TrailSource;
  kind: TrailKind;
  level: TrailLevel;
  summary: string;
  detail: unknown | null;
}

/** Per-session task roll-up for the multi-agent overview (sidebar chips). */
export interface SessionActivitySummary {
  session_id: Id;
  total: number;
  done: number;
  in_progress: string | null;
  last_ts: string | null;
}

export type TaskStatus = 'pending' | 'in_progress' | 'completed' | 'blocked' | 'cancelled';

/** One task in a session's normalized task tracker. */
export interface AgentTask {
  id: Id;
  session_id: Id;
  workspace_id: Id;
  ext_id: string | null;
  title: string;
  status: TaskStatus;
  position: number;
  /** `'agent'` = published by the agent's own task tool (TodoWrite/TaskCreate);
   *  `'user'` = added from the board / Activity panel (`POST /sessions/{id}/tasks`).
   *  Only `'agent'` rows are replaced on sync — user rows are merged by title / ext_id. */
  source: 'agent' | 'user';
  /** Free-text detail for user-added tasks (sent along in the board nudge). */
  description: string | null;
  /** True while a user-added task still waits to be nudged into the agent's PTY
   *  (the sweep delivers it once the session is idle, or after the 120 s max defer). */
  nudge_pending: boolean;
  /** When the board nudge was delivered into the PTY (null until then). */
  nudged_at: string | null;
  created_at: string;
  updated_at: string;
}

export interface AppendTrailReq {
  source?: TrailSource;
  kind?: TrailKind;
  /** Default `info`. */
  level?: TrailLevel;
  summary: string;
  detail?: unknown;
}

export interface TaskInput {
  ext_id?: string | null;
  title: string;
  status: TaskStatus;
}

export interface PutTasksReq {
  tasks: TaskInput[];
}
