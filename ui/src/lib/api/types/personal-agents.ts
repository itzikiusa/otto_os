// Contract mirror: personal-agents. Part of lib/api/types (see ../types.ts, the barrel).
// Mirrors docs/contracts/api.md + ws.md — change them together.

import type {
  Id,
} from './core';

// ---------------------------------------------------------------------------
// Personal Agents (mirror of otto_state::personal_agents — keep in lockstep)
// ---------------------------------------------------------------------------

/** A named persistent agent: persona (soul), pinned provider+model, 1..N
 *  schedules, optional browser use, per-agent delivery + memory folder, and a
 *  chat-anytime session. */
export interface PersonalAgent {
  id: Id;
  workspace_id: Id;
  name: string;
  /** Emoji / short glyph shown on the card; '' = initial of the name. */
  avatar: string;
  /** The persona, materialized into the agent's cwd as its CLAUDE.md/AGENTS.md. */
  soul_md: string;
  provider: string;
  /** '' = provider default model. */
  model: string;
  /** '' = `data_dir/personal/<agent-id>/` (resolved by the engine). */
  cwd: string;
  /** Reconcile the otto-browser Playwright MCP into every run/chat. */
  browser: boolean;
  /** `{type:'none'|'slack'|'telegram'|'email'|'webhook', chat_id?/to?/url?}` —
   *  same shape as a scheduled task's destination. */
  delivery: Record<string, unknown>;
  enabled: boolean;
  /** The agent's single interactive chat session (set by the chat-session route). */
  chat_session_id?: string | null;
  created_by?: string | null;
  created_at: string;
  updated_at: string;
}

/** One cadence+directive of a personal agent (1..N per agent, own cursor). */
export interface PersonalAgentSchedule {
  id: Id;
  agent_id: Id;
  /** Existing cadence format: `{cadence:'interval'|'daily'|'weekly'|'cron', …}`,
   *  or the one-shot `{cadence:'once', run_at}` (RFC3339 or local
   *  `YYYY-MM-DDTHH:MM` in `timezone`; the schedule disables itself after its run). */
  schedule: Record<string, unknown>;
  timezone: string;
  /** The run's task prompt for this schedule. */
  directive: string;
  enabled: boolean;
  last_run_at?: string | null;
  next_run_at?: string | null;
  /** When the schedule (or its agent) was last (re)armed — see `ScheduledTask.armed_at`. */
  armed_at?: string | null;
  /** The schedule's own permission set: `read_only` runs can't change anything. */
  permission?: PersonalAgentPermission;
  created_at: string;
  updated_at: string;
}

/** One execution of a personal agent (mirrors ScheduledTaskRun + schedule_id). */
export interface PersonalAgentRun {
  id: Id;
  agent_id: Id;
  schedule_id?: Id | null;
  workspace_id: Id;
  /** `canceled`: stopped from Otto (`POST …/runs/{run_id}/cancel`). */
  status: 'running' | 'ok' | 'error' | 'canceled';
  trigger: 'schedule' | 'manual' | 'proactive';
  started_at: string;
  finished_at?: string | null;
  summary: string;
  report_path?: string | null;
  report_rel?: string | null;
  delivered: boolean;
  delivery_error?: string | null;
  error?: string | null;
  /** The visible agent session the run drove (Open it from the run row). */
  session_id?: string | null;
  report_hash?: string | null;
  attempts: number;
  /** Delivery was suppressed because the report didn't meaningfully change. */
  skipped_delivery: boolean;
  /** The permission mode the run executed under. */
  mode?: PersonalAgentRunMode;
  /** True when the run's session was confined read-only. */
  read_only?: boolean;
  /** The standing goal a proactive run worked on. */
  goal_id?: string | null;
  created_at: string;
}

// ---- Personal agent autonomy (batch 2026-10-03) — docs/contracts/api.md
// "Personal agent autonomy".

/** A schedule's permission set. */
export type PersonalAgentPermission = 'read_only' | 'directed';
/** The mode a run executed under. */
export type PersonalAgentRunMode = 'proactive' | 'directed' | 'scheduled';

export interface PersonalAgentProactive {
  enabled: boolean;
  /** Proactive runs in any rolling 24 h (1..24). */
  runs_per_day: number;
  /** Wall-clock cap per proactive run, minutes (1..60). */
  max_minutes: number;
}

export interface PersonalAgentGoal {
  id: string;
  text: string;
  enabled: boolean;
  /** Server-maintained. */
  last_run_at: string | null;
}

export interface PersonalAgentRule {
  id: string;
  text: string;
  /** Server-derived from the text; null = instructions only. */
  enforce: { kind: 'approval' | 'deny'; terms: string[] } | null;
}

export interface PersonalAgentAutonomy {
  proactive: PersonalAgentProactive;
  goals: PersonalAgentGoal[];
  rules: PersonalAgentRule[];
  /** The workspace's primary assistant ("your agent"). */
  primary: boolean;
}

/** `PUT /personal-agents/{id}/autonomy` — partial; omitted sections are kept. */
export interface SavePersonalAgentAutonomyReq {
  proactive?: PersonalAgentProactive;
  goals?: { id?: string; text: string; enabled?: boolean }[];
  rules?: { id?: string; text: string }[];
  primary?: boolean;
}

export type PersonalAgentActivityKind =
  | 'tool_call'
  | 'blocked'
  | 'approval_required'
  | 'approval_waiting';

export interface PersonalAgentActivityItem {
  seq: number;
  at: string;
  kind: PersonalAgentActivityKind;
  tool: string;
  detail: string;
  session_id: string | null;
  approval_id: string | null;
}

export interface PersonalAgentActivityApproval {
  approval_id: string;
  tool: string;
  at: string;
  status: string;
  title: string;
  detail: string | null;
  risk_label: string | null;
}

export interface PersonalAgentActivity {
  now: { run: PersonalAgentRun | null; session_status: string | null };
  /** Newest first. With `?after_seq=N`, only entries with `seq > N`. */
  items: PersonalAgentActivityItem[];
  approvals: PersonalAgentActivityApproval[];
  /** `null` when asked with `?runs=false` (keep the ones you have). Run
   *  `summary` is clipped to 280 chars here. */
  runs: PersonalAgentRun[] | null;
  /** Cursor: the newest ring `seq` the answer covers (pass as `after_seq`). */
  seq?: number;
  /** The daemon boot id the cursor belongs to (pass back as `epoch`). The
   *  ring and its `seq` restart with the daemon; a different epoch means the
   *  cursor and items are from a previous process. */
  epoch?: string;
  /** The `after_seq`/`epoch` sent were stale (previous daemon process): this
   *  is a full answer — replace the items and the cursor. */
  reset?: boolean;
}

export type PersonalAgentMemorySource =
  | 'chat'
  | 'slack'
  | 'telegram'
  | 'vault'
  | 'run'
  | 'user'
  | 'notes';

export interface PersonalAgentMemoryItem {
  line: number;
  text: string;
  source: PersonalAgentMemorySource;
  section: string;
  /** The stored line — quote it back on edit/forget. */
  raw: string;
}

export interface PersonalAgentMemories {
  version: string;
  exists: boolean;
  path: string | null;
  items: PersonalAgentMemoryItem[];
}

/** An agent room — the ONLY agent-to-agent transport, always user-visible. */
export interface AgentRoom {
  id: Id;
  workspace_id: Id;
  name: string;
  created_by?: string | null;
  created_at: string;
  updated_at: string;
}

/** Room + its member agent ids, as the list/get routes return it. */
export interface AgentRoomWithMembers {
  room: AgentRoom;
  members: Id[];
  /** Messages in the room (0 for an empty room). */
  message_count: number;
  /** `created_at` of the newest message; null for an empty room. */
  last_message_at: string | null;
}

/** One persisted room message. `author_id` is a personal-agent id for
 *  `author_kind:'agent'`, a user id for `'user'`. */
export interface AgentRoomMessage {
  id: Id;
  room_id: Id;
  author_kind: 'agent' | 'user';
  author_id: Id;
  text: string;
  created_at: string;
}
