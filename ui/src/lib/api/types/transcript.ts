// Contract mirror: transcript. Part of lib/api/types (see ../types.ts, the barrel).
// Mirrors docs/contracts/api.md + ws.md — change them together.

// ── Transcript (conversation view · docs/design/conversation-view.md §3) ─────
// Mirror of crates/otto-transcript/src/model.rs. The conversation is rebuilt from
// the provider's own transcript on disk (Claude JSONL / Codex rollout), never
// from PTY scrollback, so these shapes are what the parser guarantees.

export type TranscriptProvider = 'claude' | 'codex' | 'agy';

/** Query for POST /sessions/{id}/transcript/touch. */
export interface TranscriptTouchQuery {
  /** Keepalive only: never resume a dormant process. Server default: false. */
  view?: boolean;
}

/** Why a session has no renderable transcript (returned with `turns: []`, 200 not 404). */
export type TranscriptUnavailableReason =
  | 'no_provider_session_id'
  | 'transcript_missing'
  | 'provider_unsupported'
  | 'codex_rollout_unresolved';

export interface TranscriptStats {
  turns: number;
  tool_calls: number;
  cost_usd: number | null;
  input_tokens: number | null;
  output_tokens: number | null;
  duration_ms: number | null;
  /** Codex reasoning items — never recoverable, so counted, not rendered. */
  reasoning_steps: number;
  /** Claude `thinking` blocks — persisted as empty markers only. */
  thinking_steps: number;
  unknown_records: number;
}

/** One node of the subagent tree (`<sid>/subagents/*.meta.json`); depth-2/3
 *  agents spawned by sibling subagents are reachable only through this list. */
export interface SubagentMeta {
  agent_id: string;
  parent_agent_id: string | null;
  depth: number;
  agent_type: string;
  description: string;
  model: string | null;
  /** The parent turn's `Agent` tool_use id this subagent hangs off. */
  tool_use_id: string | null;
}

export interface Transcript {
  session_id: string | null;
  provider: TranscriptProvider;
  title: string | null;
  cwd: string | null;
  model: string | null;
  /** Opaque `"<record_index>"` of the FIRST record of the OLDEST returned turn —
   *  pass it back as `before` (exclusive) to page earlier. Records are
   *  append-only so the index is stable; the live tail uses the WS delta's
   *  `cursor` (index of the LAST folded record). */
  cursor: string;
  /** True when `before` can page further back ("Load earlier"). */
  has_earlier: boolean;
  turns: Turn[];
  stats: TranscriptStats;
  /** FULL tree on the newest page; `?sub=` and earlier (`before`) pages
   *  return `[]` — keep the first page's. */
  subagents: SubagentMeta[];
  /** Set (with `turns: []`) when no transcript resolves for the session. */
  unavailable_reason: TranscriptUnavailableReason | null;
}

export type TurnRole = 'user' | 'assistant';

/** Token usage summed over one assistant turn's API calls (Claude
 *  `message.usage`, once per response; Codex the growth of `token_count`
 *  totals). `thinking_tokens` is the part of `output_tokens` spent on
 *  reasoning (Claude `output_tokens_details`, Codex `reasoning_output_tokens`),
 *  so the visible answer is `output_tokens - thinking_tokens`. Cache fields are
 *  separate from `input_tokens` (Claude reports them so; Codex's cached input
 *  is split out to match). */
export interface TurnUsage {
  input_tokens: number;
  output_tokens: number;
  thinking_tokens: number;
  cache_read_tokens: number;
  cache_creation_tokens: number;
}

export interface Turn {
  /** Claude: assistant → requestId (else uuid); user → uuid. Codex new era →
   *  `turn_id + ":u"|":a"`; old era → `"r<record_index>"`. Stable across re-parses. */
  id: string;
  role: TurnRole;
  ts: string | null;
  blocks: Block[];
  duration_ms: number | null;
  model: string | null;
  /** Attachments / reminders / hooks — collapsed into one muted chip per turn. */
  system: SystemNote[];
  /** Codex reasoning items in this turn (never recorded → counted only; 0 for Claude). */
  reasoning_steps: number;
  /** Assistant turns only; absent/null when the provider recorded no usage. */
  usage?: TurnUsage | null;
}

export type ToolKind =
  | 'shell'
  | 'read'
  | 'edit'
  | 'write'
  | 'search'
  | 'agent'
  | 'mcp'
  | 'skill'
  | 'web'
  | 'ask'
  | 'task'
  | 'other';

export interface ToolResult {
  ok: boolean;
  /** Capped at 64 KB (`truncated: true`). */
  text: string | null;
  truncated: boolean;
  /** Only in a live `transcript_appended` delta that would exceed 64 KB:
   *  `text` (then `patch`) was shortened for the push; the full result is
   *  `GET /sessions/{id}/transcript/tool/{tool_id}`. `bytes`/`truncated`
   *  still describe the stored result. Absent everywhere else. */
  elided?: boolean;
  bytes: number;
  /** Images extracted from the result — `GET …/transcript/images/{id}`. */
  image_ids: string[];
  /** Unified diff for edit-like tools (fed to `git/DiffViewer`). */
  patch: string | null;
  file_path: string | null;
}

export type TaskItemStatus = 'pending' | 'in_progress' | 'completed';

export interface TaskItem {
  ext_id: string | null;
  title: string;
  status: TaskItemStatus;
  active_form: string | null;
}

export type SystemNoteKind =
  | 'system_reminder'
  | 'task_notification'
  | 'command'
  | 'hook'
  | 'attachment'
  | 'compaction'
  | 'other';

export interface SystemNote {
  kind: SystemNoteKind;
  title: string;
  body: string | null;
}

export type TranscriptArtifactKind = 'file' | 'pr' | 'image' | 'report' | 'url';

export interface Artifact {
  /** sha1(kind + ':' + (path ?? url)) — opaque, stable, the ONLY handle
   *  `GET /sessions/{id}/artifacts/{artifact_id}` accepts. */
  id: string;
  kind: TranscriptArtifactKind;
  label: string;
  path: string | null;
  url: string | null;
  mime: string | null;
  produced_at: string | null;
  /** The last turn that produced this path (dedup is per path). */
  turn_id: string;
}

export type SubagentStatus = 'running' | 'done' | 'error';

export type Block =
  | { kind: 'text'; md: string }
  /** Marker only — no thinking text is persisted on disk. */
  | { kind: 'thinking'; count: number }
  | { kind: 'image'; id: string; media_type: string; alt: string | null }
  | {
      kind: 'tool_call';
      id: string;
      name: string;
      tool: ToolKind;
      title: string;
      input: unknown;
      result: ToolResult | null;
    }
  /** Children come from `Transcript.subagents[]`; the body is lazy via `?sub=<agent_id>`. */
  | {
      kind: 'subagent';
      agent_id: string;
      description: string;
      agent_type: string;
      status: SubagentStatus | null;
    }
  /** Task-list state AFTER the call. */
  | { kind: 'tasks'; tasks: TaskItem[] }
  /** Claude `queue-operation`; `injected` = a <task-notification>/system payload. */
  | { kind: 'queued'; op: 'enqueue' | 'dequeue' | 'remove'; text: string; injected: boolean }
  | { kind: 'artifact'; artifact: Artifact }
  | { kind: 'notice'; note: SystemNote };

export type HistoryStatus = 'running' | 'working' | 'idle' | 'exited' | 'reconnectable' | 'on_disk';

/** Bounded metadata page; a non-null cursor can accompany an empty result. */
export interface HistoryPage {
  entries: HistoryEntry[];
  next_cursor: string | null;
}

/** One history row: an Otto session or an unclaimed on-disk transcript. */
export interface HistoryEntry {
  session_id: string | null;
  provider: 'claude' | 'codex';
  title: string | null;
  first_prompt: string | null;
  cwd: string;
  repo_name: string | null;
  started_at: string;
  last_active_at: string;
  turns: number | null;
  status: HistoryStatus;
  transcript_path: string;
  /** The CLI left resumable state (a provider session id), whatever the
   *  status — an ended conversation resumes too. Archived rows stay true;
   *  resume refuses them (409) until unarchived. */
  resumable: boolean;
  /** Archived Otto row (false for `on_disk`; absent from older daemons). */
  archived?: boolean;
}

export interface HistoryQuery {
  q?: string;
  provider?: 'claude' | 'codex';
  cwd?: string;
  status?: HistoryStatus;
  before?: string;
  limit?: number;
}

export interface HistoryImportReq {
  provider: 'claude' | 'codex';
  transcript_path: string;
}

/** `POST /sessions/{id}/tasks` — a board / Activity-panel task pushed to the agent. */
export interface CreateAgentTaskReq {
  title: string;
  description?: string;
}

/** `POST /sessions/{id}/inbox` — image/* only, 10 MB, stored under
 *  `<data>/sessions/<id>/inbox/`; the composer inserts `[Image: <path>]`. */
export interface InboxUploadReq {
  filename: string;
  mime: string;
  data_b64: string;
}

export interface InboxUploadResp {
  path: string;
}
