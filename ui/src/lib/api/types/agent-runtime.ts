// Contract mirror: agent-runtime. Part of lib/api/types (see ../types.ts, the barrel).
// Mirrors docs/contracts/api.md ("Structured agent runtime") + ws.md
// (`agent_runtime_event`) — change them together.

// ── Structured agent runtime (docs/features/threads-structured-runtime.md) ──
// Mirror of crates/otto-core/src/agent_runtime.rs, itself a port of T3 Code's
// provider runtime model (MIT, Copyright (c) 2026 T3 Tools Inc.:
// `ProviderAdapterV2Event` + the `OrchestrationV2*` entities). Events are
// entity upserts — an item is re-sent WHOLE with its accumulated text and a
// `streaming` flag; `command_output.delta` is the only true delta. Field
// names are snake_case where T3 Code uses camelCase.

import type { Id } from './core';

export type RuntimeProvider = 'claude' | 'codex' | 'gemini' | 'grok' | 'agy' | 'opencode' | 'pi';

export type ProviderSessionStatus = 'starting' | 'ready' | 'running' | 'waiting' | 'stopped' | 'error';

export interface ProviderSession {
  status: ProviderSessionStatus;
  cwd: string;
  model: string | null;
  updated_at: string;
  last_error: string | null;
}

export interface ThreadTokenUsage {
  used_tokens: number;
  max_tokens?: number;
  input_tokens?: number;
  cached_input_tokens?: number;
  output_tokens?: number;
}

export interface ProviderThread {
  /** The provider's own session / thread id (resume key). */
  native_thread_id: string | null;
  context_usage?: ThreadTokenUsage;
  updated_at: string;
}

export type ProviderTurnStatus = 'pending' | 'running' | 'completed' | 'interrupted' | 'failed' | 'cancelled';

export interface TurnTokenUsage {
  input_tokens: number;
  output_tokens: number;
  cached_input_tokens?: number;
  reasoning_tokens?: number;
}

export interface ProviderTurn {
  id: Id;
  ordinal: number;
  status: ProviderTurnStatus;
  started_at: string | null;
  completed_at: string | null;
  turn_token_usage?: TurnTokenUsage;
}

export type TurnItemStatus =
  | 'idle' | 'pending' | 'running' | 'waiting' | 'completed' | 'failed' | 'cancelled' | 'interrupted';

export type PlanStepStatus = 'pending' | 'running' | 'completed';

export interface PlanStep {
  id: string;
  text: string;
  status: PlanStepStatus;
}

export type RequestKind = 'command' | 'file_read' | 'file_change' | 'mcp_elicitation' | 'permission';

export type ApprovalDecision = 'accept' | 'accept_for_session' | 'accept_always' | 'decline' | 'cancel';

export interface ApprovalOption {
  decision: ApprovalDecision;
  label: string;
}

export interface ProviderFailure {
  /** e.g. `provider_error`, `transport_error`, `protocol_error`. */
  class: string;
  message: string;
  code?: string;
  retryable?: boolean;
}

/** The type-specific part of a turn item. */
export type TurnItemKind =
  | { type: 'user_message'; text: string; attachments?: MessageAttachment[] }
  | { type: 'assistant_message'; message_id: string; text: string; streaming: boolean }
  | { type: 'reasoning'; text: string; streaming: boolean }
  | { type: 'todo_list'; plan_id: string; steps: PlanStep[]; explanation?: string }
  /* Tool items carry `error` (the tool's error text, ~2 KB max) when failed. */
  | { type: 'file_change'; file_name: string; diff_str?: string; old_str?: string; new_str?: string; error?: string }
  | { type: 'command_execution'; input: string; output?: string; exit_code?: number; error?: string }
  | { type: 'dynamic_tool'; tool_name: string; input: unknown; output?: string; error?: string }
  /** Web search / fetch tools; `patterns` are the queries or URLs. */
  | { type: 'web_search'; patterns?: string[]; error?: string }
  | {
      type: 'approval_request';
      request_id: Id;
      request_kind: RequestKind;
      prompt?: string;
      /** The provider's own justification (Codex `reason`). */
      reason?: string;
      options: ApprovalOption[];
      /** The turn item id of the tool call it gates, when known. */
      tool_item_id?: string;
      /** Set when cancelled; `agent` = the provider withdrew it. */
      cancel_reason?: RequestCancelReason;
    }
  | { type: 'system_notice'; message: string }
  | { type: 'error'; failure: ProviderFailure };

export type TurnItem = {
  id: Id;
  turn_id: Id | null;
  /** Display order within the session. */
  ordinal: number;
  status: TurnItemStatus;
  title: string | null;
  started_at: string | null;
  completed_at: string | null;
  updated_at: string;
} & TurnItemKind;

/** Why a request was cancelled: `agent` = the provider withdrew it (Claude
 *  does when a parallel sibling ends), `interrupt` = the turn was stopped,
 *  `closed` = the session closed. */
export type RequestCancelReason = 'agent' | 'interrupt' | 'closed';

/** A file sent with a `user_message` (bytes at
 *  GET /runtime-sessions/{id}/attachments/{attachment_id}). */
export interface MessageAttachment {
  id: string;
  kind: 'image' | 'file';
  mime: string;
  name?: string;
}

export type RuntimeRequestStatus = 'pending' | 'resolved' | 'expired' | 'cancelled';

export type ResponseCapability = { type: 'live' } | { type: 'not_resumable'; reason: string };

export interface RuntimeRequest {
  id: Id;
  turn_id: Id | null;
  kind: RequestKind;
  status: RuntimeRequestStatus;
  response_capability: ResponseCapability;
  created_at: string;
  resolved_at: string | null;
  decision?: ApprovalDecision;
  cancel_reason?: RequestCancelReason;
}

export interface PlanArtifact {
  id: string;
  turn_id: Id | null;
  steps: PlanStep[];
  explanation?: string;
}

export type TurnTerminalStatus = 'completed' | 'interrupted' | 'cancelled' | 'failed';

/** The `type`-tagged payload of a runtime event (T3 Code tags). */
export type RuntimeEventBody =
  | { type: 'provider_session.updated'; provider_session: ProviderSession }
  | { type: 'provider_thread.updated'; provider_thread: ProviderThread }
  | { type: 'provider_turn.updated'; provider_turn: ProviderTurn }
  | { type: 'turn_item.updated'; turn_item: TurnItem }
  | { type: 'runtime_request.updated'; runtime_request: RuntimeRequest }
  | { type: 'plan.updated'; plan: PlanArtifact }
  | { type: 'command_output.delta'; item_id: Id; chunk: string }
  | {
      type: 'turn.terminal';
      turn_id: Id;
      status: TurnTerminalStatus;
      failure?: ProviderFailure;
      reason?: string;
    };

export type RuntimeEventType = RuntimeEventBody['type'];

/** One sequenced event: envelope fields flattened beside the payload. */
export type RuntimeEvent = {
  session_id: Id;
  /** Per session, strictly increasing, never reused (also across restarts). */
  sequence: number;
  ts: string;
} & RuntimeEventBody;

export interface RuntimeSnapshot {
  provider_session: ProviderSession | null;
  provider_thread: ProviderThread | null;
  turns: ProviderTurn[];
  /** Ordered by `ordinal`. */
  turn_items: TurnItem[];
  runtime_requests: RuntimeRequest[];
  plans: PlanArtifact[];
}

/** Response of GET /runtime-sessions/{id}/events?after_sequence=. */
export type RuntimeReplay =
  | { kind: 'events'; events: RuntimeEvent[]; last_sequence: number }
  | { kind: 'snapshot'; snapshot_sequence: number; snapshot: RuntimeSnapshot };

export type RuntimeSessionStatus =
  | 'starting' | 'idle' | 'working' | 'awaiting_permission' | 'dormant' | 'closed';

export interface RuntimeSession {
  id: Id;
  workspace_id: Id;
  provider: RuntimeProvider;
  title: string;
  cwd: string;
  model?: string;
  native_thread_id?: string;
  status: RuntimeSessionStatus;
  access_mode: RuntimeAccessMode;
  created_by: Id;
  created_at: string;
  last_active_at: string;
  last_sequence: number;
  /** The linked group's ROOT thread, when an agent opened this thread from
   *  another one (`otto.thread_open`); absent on a root / unlinked thread. */
  parent_id?: Id | null;
  /** The agent-UI-control grant (`POST /sessions/{id}/ui-control`); absent = never decided. */
  ui_control?: { enabled: boolean; granted_at?: string; granted_by?: Id } | null;
}

/** Body of `POST /runtime-sessions/{id}/peers` — open a linked thread. */
export interface OpenPeerReq {
  provider: RuntimeProvider;
  prompt?: string;
  title?: string;
  /** Omitted = the provider's default model. */
  model?: string;
  /** Opening-turn effort; omitted = the model's default. */
  effort?: string;
}

/** One member of a linked group (`GET /runtime-sessions/{id}/peers`). */
export interface PeerInfo {
  id: Id;
  provider: RuntimeProvider;
  title: string;
  status: RuntimeSessionStatus;
  is_self: boolean;
}

/** `GET /runtime-sessions/{id}/peers/{peer}/reply?wait_secs=`. */
export interface PeerReply {
  peer: PeerInfo;
  busy: boolean;
  turn_status?: string;
  text: string;
}

/** T3 Code's RuntimeMode names; sticky per session like `model`. */
export type RuntimeAccessMode = 'approval-required' | 'auto-accept-edits' | 'full-access';

/** Body of POST /workspaces/{id}/runtime-sessions. */
export interface CreateRuntimeSessionReq {
  provider: RuntimeProvider;
  /** Absolute; defaults to the workspace root. */
  cwd?: string | null;
  title?: string | null;
  model?: string | null;
  access_mode?: RuntimeAccessMode;
}

/** Body of POST /runtime-sessions/{id}/prompt. */
/** `PATCH /runtime-sessions/{id}` — rename. */
export interface UpdateRuntimeSessionReq {
  title: string;
}

/** `text` may be empty when `images` is not. `model` / `effort` apply to this
 *  turn and stick for later ones; `effort` is Claude / Codex only. Images:
 *  Claude, Codex, and ACP agents that advertise them; Antigravity is text
 *  only. Unsupported input is a 400. */
export interface RuntimePromptReq {
  text: string;
  model?: string;
  effort?: string;
  images?: PromptImage[];
  /** Non-image files (`application/pdf` only); providers that can't take
   *  them answer 400. */
  files?: PromptFile[];
  /** Applies from this turn on and sticks. */
  access_mode?: RuntimeAccessMode;
}

export interface PromptFile {
  mime: string;
  name: string;
  data_base64: string;
}

/** An inline image (`mime` is `image/*`, base64 without a `data:` prefix). */
export interface PromptImage {
  mime: string;
  data_base64: string;
}

export interface RuntimePromptResp {
  turn_id: Id;
  /** `sequence` of the turn's first `provider_turn.updated`. */
  sequence: number;
}

/** Response of POST /runtime-sessions/{id}/interrupt. */
export interface RuntimeInterruptResp {
  interrupted: boolean;
}

/** Body of POST /runtime-sessions/{id}/requests/{request_id}. */
export interface RuntimeRequestRespondReq {
  decision: ApprovalDecision;
}
