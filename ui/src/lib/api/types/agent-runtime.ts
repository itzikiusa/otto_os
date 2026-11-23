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
import { sessionProviderSlug as resolveSessionProviderSlug } from '../../providerRegistry';

export type RuntimeProvider = 'claude' | 'codex' | 'gemini' | 'grok' | 'agy' | 'opencode' | 'pi' | 'muse';

/** Built-in `RuntimeProvider` id or a custom `ProviderRegistry` slug (e.g. `'claudework'`). */
export type RuntimeProviderOrSlug = RuntimeProvider | (string & {});

export type ProviderSessionStatus = 'starting' | 'ready' | 'running' | 'waiting' | 'stopped' | 'error';

export interface ProviderSession {
  status: ProviderSessionStatus;
  cwd: string;
  model: string | null;
  updated_at: string;
  last_error: string | null;
  /** Background work the agent started that has not finished (absent = none).
   *  The thread is busy while any is pending, even between turns. */
  background_tasks?: BackgroundTask[];
  /** What the running driver reported it takes (after its handshake). */
  capabilities?: RuntimeCapabilities;
  /** The thread's title: a rename, or the agent's own name for it. */
  title?: string;
  /** The provider the thread runs on now and its slug (a named account's
   *  own): a provider switch changes them. */
  provider?: RuntimeProvider;
  provider_slug?: string;
  /** The client-owned thread meta (`ThreadMetaPatch` keys); absent = none set. */
  thread?: ThreadMetaPatch;
}

/** What a provider switch handed the new provider (`session_boundary.handoff`):
 *  whole turns, newest first within a budget; `text` is the block as the
 *  agent read it. */
export interface ContextHandoff {
  from_provider: string;
  to_provider: string;
  from_model?: string;
  to_model?: string;
  turns: number;
  omitted_turns: number;
  images_omitted: number;
  text: string;
}

/** What a thread's agent takes besides plain text — the provider baseline
 *  until a running driver reports its own. Gate Send now (steer),
 *  attachments and effort on this, never on the provider's name. */
export interface RuntimeCapabilities {
  /** A message can join the RUNNING turn (`delivery: steer`); else it queues. */
  steer: boolean;
  images: boolean;
  /** PDFs. */
  files: boolean;
  /** Effort levels a prompt may carry; absent / empty = none. */
  efforts?: string[];
  /** A prompt may ask for plan mode (`plan: true`). */
  plan?: boolean;
}

/** One of the agent's own slash commands (`commands.updated`). */
export interface ProviderCommand {
  /** Without the leading `/`. */
  name: string;
  description?: string;
  /** What to type after it, when it takes input. */
  hint?: string;
}

/** One MCP server a thread's agent has (`mcp_servers.updated`): Otto's own
 *  tools, a control-plane server Otto handed it, or one from the CLI's own
 *  config — set at spawn, refined by what the CLI reports. */
export interface McpServerStatus {
  name: string;
  source: 'otto' | 'workspace' | 'cli';
  /** `added`: given to the agent; the CLI reports no connection status. */
  state: 'connected' | 'starting' | 'added' | 'needs_auth' | 'failed' | 'disabled' | 'unsupported';
  /** Why it is not connected, in words. */
  reason?: string;
}

/** `GET /runtime-sessions/{id}/tasks/{task_id}/output` — a Claude background
 *  command's output so far (the tail). */
export interface RuntimeTaskOutput {
  text: string;
  truncated: boolean;
}

/** `GET /runtime-sessions/{id}/swarm?member=` — an Antigravity swarm. */
export interface RuntimeSwarm {
  title?: string;
  members: RuntimeSwarmMember[];
}
export interface RuntimeSwarmMember {
  name: string;
  role: string;
  phase?: string;
  killed: boolean;
  model?: string;
  workspace?: string;
  posts: { seq: number; agent: string; text: string; time?: string }[];
  /** Only for the member asked for. */
  steps?: { index: number; type: string; status?: string; text?: string; tools?: string[] }[];
  truncated: boolean;
}

/** One piece of background work (a `run_in_background` command, a
 *  background subagent, or a one-shot timer). */
export interface BackgroundTask {
  id: string;
  /** `command`, `agent`, `timer`, or the provider's own task type. */
  kind: string;
  label: string;
  started_at: string;
  /** The tool call that started it, when known. */
  item_id?: Id;
  /** Optional scheduled fire timestamp for one-shot timers (`schedule`). */
  due_at?: string;
  /** A stop was asked for and the provider has not settled the task yet
   *  (absent on a daemon without per-task stop). */
  stopping?: boolean;
  /** Why the last stop did not take (the task is still listed). */
  stop_error?: string | null;
}

/** How `POST …/tasks/{task_id}/stop` stopped it: the provider's own stop,
 *  an OS signal, or Otto only ASKED the agent to stop it (the task stays
 *  listed until the agent's own settlement). */
export type RuntimeTaskStopOutcome = 'native' | 'signal' | 'asked';

/** Response of POST /runtime-sessions/{id}/tasks/{task_id}/stop. */
export interface RuntimeTaskStopResp {
  task_id: string;
  outcome: RuntimeTaskStopOutcome;
}

/** Response of POST /runtime-sessions/{id}/tasks/stop (every running task). */
export interface RuntimeTasksStopResp {
  results: ({ task_id: string; outcome: RuntimeTaskStopOutcome } | { task_id: string; error: string })[];
}

/** Structured settlement metadata on `system_notice` when background work finishes. */
export interface TaskSettlement {
  task_id: string;
  kind: string;
  /** `'completed' | 'stopped' | 'failed'` */
  status: string;
  label: string;
}

/** Who started a turn; `agent` = the agent continued on its own (its
 *  background work finished). Absent = `user`. */
export type TurnOrigin = 'user' | 'agent';

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
  /** What the turn cost in USD, where the provider says. */
  cost_usd?: number;
}

export interface ProviderTurn {
  id: Id;
  ordinal: number;
  status: ProviderTurnStatus;
  started_at: string | null;
  completed_at: string | null;
  turn_token_usage?: TurnTokenUsage;
  origin?: TurnOrigin;
  /** The model / effort this turn actually ran on (absent on older daemons). */
  model?: string | null;
  effort?: string | null;
}

export type TurnItemStatus =
  | 'idle' | 'pending' | 'running' | 'waiting' | 'completed' | 'failed' | 'cancelled' | 'interrupted';

export type PlanStepStatus = 'pending' | 'running' | 'completed';

export interface PlanStep {
  id: string;
  text: string;
  status: PlanStepStatus;
}

export type RequestKind = 'command' | 'file_read' | 'file_change' | 'mcp_elicitation' | 'permission' | 'question';

export type ApprovalDecision = 'accept' | 'accept_for_session' | 'accept_always' | 'decline' | 'decline_always' | 'cancel';

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
  /** `image_files`: images the tool left on disk, served by
   *  GET /runtime-sessions/{id}/items/{item_id}/images/{n}. */
  | { type: 'dynamic_tool'; tool_name: string; input: unknown; output?: string; images?: ToolOutputImage[]; image_files?: string[]; error?: string }
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
  | { type: 'session_boundary'; handoff?: ContextHandoff }
  | { type: 'system_notice'; message: string; task_settlement?: TaskSettlement }
  /** A subagent (Claude's Agent / Task tool): its own messages and tools are
   *  items whose `parent_item_id` is this item. A background one stays
   *  `running` past its turn until its task ends. */
  | {
      type: 'subagent';
      tool_name: string;
      description?: string;
      subagent_type?: string;
      prompt?: string;
      /** Its final answer (tool result / task summary). */
      output?: string;
      background: boolean;
      last_tool?: string;
      tool_uses: number;
      error?: string;
    }
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
  /** The `subagent` item this item is the activity of. */
  parent_item_id?: Id;
  /** True when heavy settled tool/diff/output fields were omitted in a compact snapshot. */
  detail_omitted?: boolean;
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
  /** Optimistic client-side data URL preview before the turn settles. */
  preview_url?: string;
}

export type RuntimeRequestStatus = 'pending' | 'resolved' | 'expired' | 'cancelled';

export type ResponseCapability = { type: 'live' } | { type: 'not_resumable'; reason: string };
export interface QuestionOption { value: string; label: string; description?: string; }
export interface RuntimeQuestion {
  id: string;
  header: string;
  prompt: string;
  kind: 'single' | 'multi' | 'text';
  required: boolean;
  allow_empty?: boolean;
  allow_free_text: boolean;
  options: QuestionOption[];
  /** Driver-set only (ACP url elicitation): opened when the person picks `open`. */ open_url?: string;
}
export type QuestionAnswer =
  | { question_id: string; type: 'single'; value: string }
  | { question_id: string; type: 'multi'; values: string[] }
  | { question_id: string; type: 'text'; text: string };

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
  questions?: RuntimeQuestion[];
  question_title?: string;
  answers?: QuestionAnswer[];
  /** An MCP / ACP elicitation: only the person answers it, never a parent agent. */ for_person?: boolean;
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
  /** The messages waiting for the running turn, whole (persisted). */
  | { type: 'queue.updated'; queued_prompts: QueuedPrompt[] }
  /** The agent's own slash commands, whole. */
  | { type: 'commands.updated'; commands: ProviderCommand[] }
  /** The MCP servers its agent has, whole. */
  | { type: 'mcp_servers.updated'; mcp_servers: McpServerStatus[] }
  | { type: 'command_output.delta'; item_id: Id; chunk: string }
  | {
      type: 'turn.terminal';
      turn_id: Id;
      status: TurnTerminalStatus;
      failure?: ProviderFailure;
      /** e.g. `stalled` (the stall watchdog stopped it), `the session was closed`. */
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
  /** Messages queued behind the running turn, oldest first. */
  queued_prompts?: QueuedPrompt[];
  /** The agent's own slash commands. */
  commands?: ProviderCommand[];
  /** The MCP servers its agent has. */
  mcp_servers?: McpServerStatus[];
  /** Set when `tail_turns` / `before_turn_ordinal` omitted earlier turns. */
  has_older?: boolean;
  /** `ordinal` of the earliest turn included in this slice, when paginated. */
  oldest_turn_ordinal?: number;
  /** Count of earlier turns omitted before `oldest_turn_ordinal`. */
  older_turns?: number;
}

/** A message waiting in a thread's queue (sent when the running turn ends). */
export interface QueuedPrompt {
  id: Id;
  text: string;
  /** Images / files riding with it. */
  attachments: number;
  created_at: string;
  /** Its last delivery failed: kept, skipped by the automatic drain until a
   *  person retries (`POST …/queue/{id}` `{delivery: "queue"}`), edits or removes it. */
  error?: string;
}

/** How a message sent while a turn runs is delivered: `queue` (after it),
 *  `steer` (into it now where `capabilities.steer`; queued elsewhere), `interrupt` (stop it
 *  and send this next). */
export type PromptDelivery = 'queue' | 'steer' | 'interrupt';

/** Body of POST /runtime-sessions/{id}/queue/{queued_id}. */
export interface QueuedDeliverReq {
  delivery: PromptDelivery;
}

/** Response of DELETE /runtime-sessions/{id}/queue/{queued_id}. */
export interface UnqueuePromptResp {
  ok: boolean;
  text?: string;
  images?: PromptImage[];
  files?: PromptFile[];
}

/** Response of GET /runtime-sessions/{id}/events?after_sequence=. */
export type RuntimeReplay =
  | { kind: 'events'; events: RuntimeEvent[]; last_sequence: number }
  | { kind: 'snapshot'; snapshot_sequence: number; snapshot: RuntimeSnapshot };

/** Response for hydrated single-item detail (`GET /runtime-sessions/{id}/items/{item_id}`). */
export type RuntimeTurnItemDetailResponse = TurnItem;

/** POST /runtime-sessions/:id/reopen revives closed/dormant sessions without a prompt; live agents are preserved. */
export type RuntimeSessionStatus =
  | 'starting' | 'idle' | 'working' | 'awaiting_permission' | 'dormant' | 'closed';

export interface RuntimeSession {
  id: Id;
  workspace_id: Id;
  provider: RuntimeProvider;
  title: string;
  cwd: string;
  model?: string;
  /** The stored reasoning effort later turns run on (absent = the agent's default). */
  effort?: string | null;
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
  /** Server-owned meta a personal agent's run / chat thread runs under
   *  (`personal_agent`, `read_only`, `source`, `run_id`, …); absent on a plain
   *  thread. A client sets only an embedded thread's `source` / `origin`, via
   *  the create body (`CreateRuntimeSessionReq.source` / `.origin`). */
  meta?: (Record<string, unknown> & {
    source?: string;
    origin?: EmbeddedOrigin;
    council?: CouncilMeta;
    provider_slug?: string;
  }) | null;
  /** Background work still pending (live; absent = none). */
  background_tasks?: BackgroundTask[];
  /** What its agent takes (see `ProviderSession.capabilities` for the live value). */
  capabilities: RuntimeCapabilities;
  /** The title is still the placeholder it was created with; the agent's own
   *  name for the conversation replaces it. */
  title_auto?: boolean;
  /** How the latest turn ended; absent / null before any turn ended and while
   *  a turn runs — a failure stays on the row until the next turn starts.
   *  Survives a daemon restart. */
  last_turn_status?: RuntimeTurnOutcome | null;
  /** Only with `last_turn_status: 'failed'`: `usage_limit`, `auth` or
   *  `provider` (absent when the failure was not recorded). */
  last_error_class?: RuntimeErrorClass | null;
}

/** `RuntimeSession.last_turn_status` (`cancelled` reads `interrupted`). */
export type RuntimeTurnOutcome = 'completed' | 'failed' | 'interrupted';
/** `RuntimeSession.last_error_class` — open-ended: treat an unknown value as `provider`. */
export type RuntimeErrorClass = 'usage_limit' | 'auth' | 'provider' | (string & {});

/** Effective provider slug of a runtime session (`meta.provider_slug` for custom
 *  providers such as `'claudework'`, or `session.provider` for built-ins). */
export function sessionProviderSlug(
  session: Pick<RuntimeSession, 'provider' | 'meta'> | null | undefined,
): string {
  return resolveSessionProviderSlug(session);
}

/** Body of `POST /runtime-sessions/{id}/peers` — open a linked thread. */
export interface OpenPeerReq {
  provider: RuntimeProviderOrSlug;
  provider_slug?: string;
  prompt?: string;
  title?: string;
  /** Omitted = the provider's default model. */
  model?: string;
  /** Opening-turn effort; omitted = the model's default. */
  effort?: string;
}

/** `meta.council` of a council thread: seat 1–4 (`0` = the synthesis). */
export interface CouncilMeta {
  id: Id;
  seat: number;
  role: 'seat' | 'synthesis';
  /** The effort the seat's (or the judge's) turn asked for. */
  effort?: string;
  /** The provider picked: a custom slug (`claudework`), else the built-in id.
   *  Absent on councils started before it was recorded. */
  provider?: string;
  /** The model picked; absent = the provider's default. */
  model?: string;
}

/** One seat of a council; the same provider may sit more than once. */
export interface CouncilSeatReq {
  provider: RuntimeProviderOrSlug;
  provider_slug?: string;
  model?: string;
  effort?: string;
}

/** Body of `POST /workspaces/{id}/runtime-sessions/council` (2–4 seats, all or none). */
export interface CreateCouncilReq {
  prompt: string;
  title?: string;
  /** Every seat's folder; omitted = the workspace root. */
  cwd?: string;
  seats: CouncilSeatReq[];
  images?: PromptImage[];
}

/** Response of the council create: seat 1 (the group's root) first. */
export interface CouncilResp {
  seats: RuntimeSession[];
}

/** `meta.child_of` of a thread opened from another one (Threads inside a
 *  thread): the opener, and whether a person or its agent opened it. */
export interface ChildOf {
  id: Id;
  by: 'person' | 'agent';
}

/** One agent of `POST /runtime-sessions/{id}/children`. */
export interface AskChildTarget {
  provider: RuntimeProviderOrSlug;
  provider_slug?: string;
  model?: string;
  effort?: string;
}

/** Body of `POST /runtime-sessions/{id}/children` ("Ask another agent"; 1–4
 *  targets, all or none). Access: omitted = the parent's; wider is a 400. */
export interface AskChildReq {
  prompt: string;
  targets: AskChildTarget[];
  title?: string;
  access_mode?: RuntimeAccessMode;
  images?: PromptImage[];
  /** "Answer to me only": the answer stays with the person (default: it
   *  wakes the thread's agent, and the subagent gets the conversation so far). */
  to_me_only?: boolean;
}

export interface AskChildResp {
  children: RuntimeSession[];
}

/** Body of `POST /runtime-sessions/{id}/council/synthesize` — the judge of
 *  each seat's latest exchange. 409 while a seat works. */
export interface SynthesizeCouncilReq {
  provider: RuntimeProviderOrSlug;
  provider_slug?: string;
  model?: string;
  effort?: string;
  /** Stop a seat's leftover background work instead of a 409. */
  stop_background?: boolean;
}

/** One member of a linked group (`GET /runtime-sessions/{id}/peers`). */
export interface PeerInfo {
  id: Id;
  provider: RuntimeProvider;
  provider_slug?: string | null;
  title: string;
  status: RuntimeSessionStatus;
  is_self: boolean;
}

/** `POST /runtime-sessions/{id}/stop-family` (body `{background}`): one Stop
 *  for the thread and its open subagents. */
export interface FamilyStopResp {
  subagents: number;
  turns: number;
  tasks: number;
}

/** `GET /runtime-sessions/{id}/subagent-targets`: a provider the thread's
 *  agent may delegate to (`otto.thread_capabilities`). */
export interface SubagentTarget {
  provider: string;
  /** Up to 12 model ids its CLI lists; empty = its default. */
  models: string[];
  efforts: string[];
}

/** `GET /runtime-sessions/{id}/peers/{peer}/reply?wait_secs=`. */
export interface PeerReply {
  peer: PeerInfo;
  /** A turn (the person's or the agent's own) or background work is running. */
  busy: boolean;
  background_tasks: number;
  turn_status?: string;
  text: string;
}

/** T3 Code's RuntimeMode names; sticky per session like `model`. */
export type RuntimeAccessMode = 'approval-required' | 'auto-accept-edits' | 'full-access';

/** `meta.source` of a thread opened inside another page (mirrors Rust
 *  `EMBEDDED_THREAD_SOURCES`); the only sources a client may name. */
export type EmbeddedThreadSource = 'pr-agent' | 'connection-diagnose';

/** `meta.origin` of an embedded thread: what it was opened for. `kind` pairs
 *  with the source (`pr-agent` ↔ `pr`, `connection-diagnose` ↔ `connection`);
 *  `id` is 1–200 characters, `label` at most 200. */
export interface EmbeddedOrigin {
  kind: 'pr' | 'connection';
  id: string;
  label?: string;
}

/** Body of POST /workspaces/{id}/runtime-sessions. */
export interface CreateRuntimeSessionReq {
  provider: RuntimeProviderOrSlug;
  /** Absolute; defaults to the workspace root. */
  cwd?: string | null;
  title?: string | null;
  /** `title` is a placeholder the agent's own name for it replaces. */
  title_auto?: boolean;
  model?: string | null;
  /** One of the provider's `capabilities.efforts`; stored as `RuntimeSession.effort`. */
  effort?: string | null;
  access_mode?: RuntimeAccessMode;
  /** A named provider account (`meta.account_id`), applied at every spawn. */
  account_id?: string;
  /** A network profile of the workspace (`meta.network_profile_id`). */
  network_profile_id?: string;
  /** Makes this an embedded thread (stored as `meta.source`); any other value
   *  is a 400. Embedded threads close after `embedded_thread_auto_close_hours`
   *  idle (default 12, 0 = never) and reopen from History. */
  source?: EmbeddedThreadSource;
  /** Stored as `meta.origin`; only with `source` (400 otherwise). */
  origin?: EmbeddedOrigin;
}

/** Query of GET /workspaces/{id}/runtime-sessions: either narrows the list to
 *  `meta.source` / `meta.origin.id` (closed threads included, newest first). */
export interface ListRuntimeSessionsQuery {
  source?: string;
  origin_id?: string;
}

/** Body of POST /runtime-sessions/{id}/prompt. */
/** `PATCH /runtime-sessions/{id}` — rename. */
/** `PATCH /runtime-sessions/{id}`: a rename and/or the client-owned thread meta. */
export interface UpdateRuntimeSessionReq {
  title?: string;
  /** A merge patch of the thread keys only (`null` clears one). */
  meta?: ThreadMetaPatch;
}

/** The client-owned Threads keys on a session's `meta` — a structured one
 *  (`PATCH /runtime-sessions/{id}`) and a terminal one (`PATCH /sessions/{id}`) alike. */
export interface ThreadMetaPatch {
  thread_pinned?: boolean | null;
  /** Moved to History. */
  thread_history?: boolean | null;
  /** RFC 3339: when the person last had the thread in front of them. */
  thread_seen_at?: string | null;
}

/** `text` may be empty when `images` is not. `model` / `effort` apply to this
 *  turn and stick for later ones (`effort` is supported by Claude, Codex, agy,
 *  and pi models that advertise reasoning levels). Images: Claude, Codex, and
 *  ACP agents that advertise them; Antigravity is text only. Unsupported input
 *  is a 400. */
export interface RuntimePromptReq {
  /** Start a fresh provider conversation in the same thread; refuses outstanding work. */
  fresh_context?: boolean;
  /** Continue the thread on another provider (a slug, built-in or a named
   *  account): a fresh conversation there, handed the thread's history
   *  (`session_boundary.handoff`). Implies `fresh_context`. */
  provider?: string;
  /** With `fresh_context`: stop the thread's background tasks first instead
   *  of refusing (an older daemon ignores it and still answers 409). */
  stop_background?: boolean;
  text: string;
  model?: string;
  effort?: string;
  images?: PromptImage[];
  /** Non-image files (`application/pdf` only); providers that can't take
   *  them answer 400. */
  files?: PromptFile[];
  /** Applies from this turn on and sticks. */
  access_mode?: RuntimeAccessMode;
  /** Plan mode for this turn (`capabilities.plan`). */
  plan?: boolean;
  /** While a turn runs; omitted = refused (409). */
  delivery?: PromptDelivery;
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
  name?: string;
}

export interface RuntimePromptResp {
  /** The turn it started or steered into; empty when queued. */
  turn_id: Id;
  /** `sequence` of the turn's first `provider_turn.updated` (or of the
   *  `queue.updated` / steered message). */
  sequence: number;
  queued_id?: Id;
  steered?: boolean;
}

/** Response of POST /runtime-sessions/{id}/interrupt. */
export interface RuntimeInterruptResp {
  interrupted: boolean;
  /** Background tasks still running (no turn ran, so nothing was stopped). */
  background_tasks?: number;
}

/** Body of POST /runtime-sessions/{id}/requests/{request_id}. */
export type RuntimeRequestRespondReq = { decision: ApprovalDecision } | { answers: QuestionAnswer[] } | { question_action: 'decline' | 'cancel' };


/** Inline raster tool result; PNG/JPEG/GIF/WebP, 1 MiB each / 2 MiB total, max 4. */
export interface ToolOutputImage {
  mime: string;
  data_base64: string;
}

/** Durable, person-scoped composer content. Never browser-persisted. */
export interface RuntimeComposerContext {
  id: string; kind: 'review-comment' | 'diff'; label: string; text: string;
  url?: string; path?: string; author?: string;
}
export interface RuntimeDraftContent {
  text: string;
  images: (PromptImage & { name: string })[];
  model: string | null | undefined;
  /** True when user explicitly picked "Default model" (`model === null`) rather than inherit (`undefined`). */
  model_default?: boolean;
  effort: import('../../providerModels').EffortLevel | null;
  access_mode: RuntimeAccessMode | null;
  fresh_context: boolean;
  /** An armed provider switch (`RuntimePromptReq.provider` of the next send). */
  provider?: string | null;
  context: RuntimeComposerContext[];
}
export interface RuntimeDraftRecord { revision: number; draft: RuntimeDraftContent | null }
