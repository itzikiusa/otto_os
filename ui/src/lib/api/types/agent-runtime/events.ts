// Contract mirror: agent-runtime (entities and events). Part of lib/api/types/agent-runtime.ts
// (split by size; see that file's header). Change with docs/contracts/api.md + ws.md.

import type { Id } from '../core';
import type { ThreadMetaPatch, PromptFile, PromptImage, ToolOutputImage } from './sessions';

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
  always?: boolean; // with "decline": really decline_always (a rollback-safe store)
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
