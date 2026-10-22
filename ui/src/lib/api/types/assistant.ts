// Contract mirror: assistant. Part of lib/api/types (see ../types.ts, the barrel).
// Mirrors docs/contracts/api.md + ws.md — change them together.

import type {
  Id,
} from './core';

// ---------------------------------------------------------------------------
// ── Otto Assistant (mirror of crates/otto-assistant/src/assistant/types.rs —
// docs/contracts/api.md "Otto Assistant"; keep in lockstep)
// ---------------------------------------------------------------------------

/** Where a request came from — reminders and replies are delivered back there. */
export type AssistantOrigin = 'app' | 'thread' | 'bar' | 'phone' | 'channel';

/** The router's request classes (local keyword rules, no LLM call). */
export type AssistantRouteKind = 'chat' | 'code' | 'hard' | 'voice';

/** Why a turn went to its provider: thread pin, `@claude`/`@codex` mention,
 *  a keyword rule, the chat default, or a (confirmed / auto) failover. */
export type AssistantRouteReason = 'pin' | 'mention' | 'rule' | 'default' | 'failover';

/** Thread list pagination: default 100, maximum 200; stable slot/update/id order. */
export interface AssistantThreadQuery { limit?: number; offset?: number }

/** One assistant thread = one resumable CLI session (resumed on demand). */
export interface AssistantThread {
  id: Id;
  /** Floating-bar space 1–4 (unique per user), or null when unslotted. */
  space_slot: 1 | 2 | 3 | 4 | null;
  title: string;
  /** The provider of the CURRENT backing session ('claude' | 'codex' | …). */
  provider: string;
  model: string | null;
  account_id: string | null;
  /** True when the user pinned provider/model (the model chip); rules are off. */
  route_pinned: boolean;
  /** The current backing session (null until the first turn). */
  session_id: Id | null;
  /** No memory reads/writes; deleted 24 h after the last turn. */
  incognito: boolean;
  /** The per-thread answer to "continue on X?" when a limit hits. */
  failover_choice: 'ask' | 'switch' | 'stay';
  /** asleep = no live session (resumed on the next send). */
  status: 'asleep' | 'idle' | 'working';
  last_turn_at: string | null;
  created_at: string;
  updated_at: string;
}

export interface AssistantAttachment {
  id: Id;
  name: string;
  /** Absolute path under the assistant's `inbox/`. */
  path: string;
  mime: string;
  size: number;
}

/** `data` of a `kind:'memory'` turn — the memory chip. */
export interface AssistantMemoryChip {
  action: 'remembered' | 'forgot' | 'pending';
  memory_ids: Id[];
  undo:
    | { kind: 'delete'; memory_id: Id }
    | { kind: 'restore'; undo_tokens: string[] }
    | null;
}

/** One indexed turn. The canonical text of a reply stays in the provider
 *  transcript; `text` here is the indexed copy (search, hand-offs, phone). */
export interface AssistantTurn {
  id: Id;
  thread_id: Id;
  role: 'user' | 'assistant' | 'system';
  kind: 'message' | 'memory' | 'delegation' | 'task' | 'reminder' | 'route' | 'limit' | 'approval';
  text: string;
  /** Provider badge (assistant turns; the routed provider on user turns). */
  provider: string | null;
  model: string | null;
  route_reason: AssistantRouteReason | null;
  session_id: Id | null;
  attachments: AssistantAttachment[];
  /** Kind-specific payload: AssistantMemoryChip for 'memory', `{task_id}` for
   *  'task' / 'reminder' / 'delegation' / 'approval', `{from, to}` for 'route'. */
  data: Record<string, unknown> | null;
  created_at: string;
}

export interface AssistantSendReq {
  text: string;
  attachment_ids?: Id[];
  origin?: AssistantOrigin;
  voice?: boolean;
}

export interface AssistantRouteTarget {
  provider: string;
  model: string | null;
  account_id: string | null;
}

export interface AssistantRouteDecision extends AssistantRouteTarget {
  kind: AssistantRouteKind;
  reason: AssistantRouteReason;
  /** Keywords / mention that decided it (for the badge tooltip). */
  matched: string[];
  /** The text that will be pasted (a leading @mention stripped). */
  text: string;
}

export interface AssistantSendResp {
  turn: AssistantTurn;
  route: AssistantRouteDecision;
  thread: AssistantThread;
}

export interface CreateAssistantThreadReq {
  title?: string;
  space_slot?: 1 | 2 | 3 | 4 | null;
  /** Given ⇒ the thread starts pinned to it. */
  provider?: string;
  model?: string | null;
  account_id?: string | null;
  incognito?: boolean;
}

export interface UpdateAssistantThreadReq {
  title?: string;
  space_slot?: 1 | 2 | 3 | 4 | null;
}

/** `POST …/route` — `provider: null` clears the pin. */
export interface AssistantRouteReq {
  provider: string | null;
  model?: string | null;
  account_id?: string | null;
}

export interface AssistantRoutingSettings {
  targets: Record<AssistantRouteKind, AssistantRouteTarget>;
  /** User keywords added to the built-in rules. */
  extra_keywords: { code: string[]; hard: string[] };
  /** Switch provider by itself when a limit hits (default false). */
  auto_failover: boolean;
  /** Agent memory writes queue for review (default false). */
  memory_approval: boolean;
  updated_at: string | null;
}

export interface AssistantLimitState {
  provider: string;
  account_id: string | null;
  limited: boolean;
  /** When the provider said the limit resets, if it said. */
  until: string | null;
  message: string;
  source: 'pty' | 'transcript' | 'probe';
  detected_at: string;
}

/** The guideline approval shape: where / what / who sees it / reason. */
export interface AssistantApprovalCard {
  where: string;
  what: string;
  who_sees: string;
  reason: string;
  tool: string | null;
  destination: string | null;
  category: 'send' | 'post' | 'publish' | 'purchase' | 'delete' | 'submit' | 'prod' | 'other';
  /** False for purchase / prod — "always allow" is never offered. */
  always_allow_allowed: boolean;
}

export interface AssistantNeedsYou {
  kind: 'approval' | 'question' | 'takeover' | 'limit' | 'memory';
  prompt: string;
  approval?: AssistantApprovalCard;
  options?: string[];
  limit?: AssistantLimitState;
  suggestion?: AssistantRouteTarget;
  memory_id?: Id;
}

export type AssistantTaskState =
  | 'queued'
  | 'running'
  | 'needs_you'
  | 'done'
  | 'failed'
  | 'cancelled';

export interface AssistantTask {
  id: Id;
  /** Agent updates require this owning thread; explicit running updates rebind its current execution. */
  thread_id: Id | null;
  kind:
    | 'task'
    | 'reminder'
    | 'approval'
    | 'question'
    | 'takeover'
    | 'limit'
    | 'delegation'
    | 'memory_review';
  state: AssistantTaskState;
  title: string;
  detail: string;
  origin: AssistantOrigin;
  /** Reminders: when it fires (UTC RFC3339). */
  run_at: string | null;
  timezone: string;
  /** A `once` Personal Agent schedule backing this task, if any. */
  schedule_id: Id | null;
  /** Delegation: the Personal Agent + its run. */
  agent_id: Id | null;
  agent_run_id: Id | null;
  needs_you: AssistantNeedsYou | null;
  result: Record<string, unknown> | null;
  created_at: string;
  updated_at: string;
  finished_at: string | null;
}

export interface AssistantCreateTaskReq {
  kind: 'task' | 'reminder';
  title: string;
  detail?: string;
  thread_id?: Id;
  /** RFC3339, or local `YYYY-MM-DDTHH:MM` in `timezone`. Required for reminders. */
  run_at?: string;
  timezone?: string;
  origin?: AssistantOrigin;
}

export type AssistantTaskAction = 'approve' | 'deny' | 'takeover' | 'handback' | 'cancel';

export interface AssistantDecisionReq {
  reason?: string;
  /** Answer to a `question` item. */
  answer?: string;
  /** Approval: remember for this destination + tool (refused for purchase/prod). */
  always_allow?: boolean;
  /** Limit item: the provider to continue on. */
  provider?: string;
}

export interface AssistantDelegateReq {
  agent_id: Id;
  directive: string;
}

export interface AssistantMemory {
  id: Id;
  text: string;
  kind: string;
  tags: string[];
  state: 'accepted' | 'pending';
  source: { kind: 'agent' | 'user' | 'hermes'; thread_id: Id | null; file: string | null };
  created_at: string;
  updated_at: string;
}

export interface AssistantProfileDoc {
  content: string;
  /** Opaque; send back with a save (409 when stale). */
  version: string;
  exists: boolean;
}

export interface AssistantMemoryView {
  profile: AssistantProfileDoc;
  memories: AssistantMemory[];
  /** Awaiting review (memory approval on, or a Hermes import). */
  pending: AssistantMemory[];
  memory_approval: boolean;
}

export interface AssistantForgetResp {
  forgotten: AssistantMemory[];
  undo_tokens: string[];
}

export interface AssistantHermesPreview {
  available: boolean;
  files: { name: string; entries: number }[];
  entries: { file: string; text: string; duplicate: boolean }[];
}

export interface AssistantHermesImportResp {
  queued: number;
  duplicates: number;
  files: string[];
}
