// Contract mirror: agent-runtime (sessions, peers and requests). Part of lib/api/types/agent-runtime.ts
// (split by size; see that file's header). Change with docs/contracts/api.md + ws.md.

import type { Id } from '../core';
import { sessionProviderSlug as resolveSessionProviderSlug } from '../../../providerRegistry';
import type { RuntimeProvider, RuntimeProviderOrSlug, ProviderSession, RuntimeCapabilities, BackgroundTask, ApprovalDecision, QuestionAnswer, PromptDelivery } from './events';

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
    /** Derived, never stored (RFC 3339): when the latest turn ended / started (kept while it runs). */
    last_turn_ended_at?: string;
    last_turn_started_at?: string;
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

/** Body of `POST /runtime-sessions/{id}/children` ("Delegate to…"; 1–4
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
  effort: import('../../../providerModels').EffortLevel | null;
  access_mode: RuntimeAccessMode | null;
  fresh_context: boolean;
  /** An armed provider switch (`RuntimePromptReq.provider` of the next send). */
  provider?: string | null;
  context: RuntimeComposerContext[];
}
export interface RuntimeDraftRecord { revision: number; draft: RuntimeDraftContent | null }
