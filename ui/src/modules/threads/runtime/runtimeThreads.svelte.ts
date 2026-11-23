// Structured runtime sessions in Threads (docs/features/threads-structured-runtime.md):
// the session rows per workspace (status kept live from `agent_runtime_event`)
// and the session actions. An open thread's conversation is runtimeView.svelte.ts,
// loaded with the view.

import { loadErrorText } from '../../../lib/loadError';
import { installRuntimeDraftRecovery } from '../../../lib/stores/runtimeDraftRecovery.svelte';
import { runtimeDrafts } from '../../../lib/stores/runtimeDrafts.svelte';
import { api } from '../../../lib/api/client';
import { appLive, onLive } from '../../../lib/live';
import { router } from '../../../lib/router.svelte';
import { noteRuntimeWaiting, syncRuntimeWaiting } from '../../../lib/stores/runtimeAttention.svelte';
import { uiControl } from '../../../lib/stores/uiControl.svelte';
import type {
  ApprovalDecision,
  AskChildReq,
  AskChildResp,
  EmbeddedOrigin,
  EmbeddedThreadSource,
  RuntimeAccessMode,
  PeerInfo,
  QuestionAnswer,
  PromptDelivery,
  ProviderSessionStatus,
  QueuedPrompt,
  RuntimeInterruptResp,
  RuntimeEvent,
  RuntimePromptReq,
  RuntimePromptResp,
  RuntimeProvider,
  RuntimeProviderOrSlug,
  RuntimeSession,
  RuntimeSessionStatus,
  Session,
  UnqueuePromptResp,
} from '../../../lib/api/types';
import { isRuntimeProvider, RUNTIME_PROVIDERS, sessionProviderSlug } from '../../../lib/providers';
import { threads } from '../threadsStore.svelte';

export { isRuntimeProvider, RUNTIME_PROVIDERS };

type QueuedAttachment = { name: string; mime: string; data_base64: string };

/** `set_session_status` in otto-agent-protocol's runtime.rs. */
const SESSION_STATUS: Record<ProviderSessionStatus, RuntimeSessionStatus> = {
  starting: 'starting',
  ready: 'idle',
  running: 'working',
  waiting: 'awaiting_permission',
  stopped: 'dormant',
  error: 'dormant',
};

/** The server-owned meta keys a Threads row keeps: who a personal agent's
 *  thread belongs to, what started it (lib/personalAgentBadges.ts), and any
 *  configured custom provider slug (`provider_slug`). The rest stays on the
 *  runtime row — a row never inherits keys that would change how other session
 *  code treats it. */
const ROW_META_KEYS = [
  'source',
  'provider_slug',
  'personal_agent',
  'personal_agent_name',
  'personal_agent_chat',
  'trigger',
  'routine',
  'agent_mode',
  'run_id',
  'read_only',
  'delegated_by_agent',
  'delegated_by_name',
] as const;

const threadSessionRowCache = new WeakMap<RuntimeSession, Session>();

/** A runtime session as the Threads sidebar's row shape (never sent to the
 *  PTY session routes — `meta.runtime` marks it). */
export function asThreadSession(r: RuntimeSession): Session {
  const cached = threadSessionRowCache.get(r);
  if (cached) return cached;
  const kept: Record<string, unknown> = {};
  for (const k of ROW_META_KEYS) if (r.meta && r.meta[k] !== undefined) kept[k] = r.meta[k];
  const mapped: Session = {
    id: r.id,
    workspace_id: r.workspace_id,
    kind: 'agent',
    provider: sessionProviderSlug(r),
    title: r.title,
    // Background work the agent started keeps the row alive between turns,
    // but calm: `running` ("Background task running"), never the working
    // pulse of a turn — the agent itself is idle.
    // A turn waiting for an approval is still a turn in flight, never "Idle".
    status: r.status === 'working' || r.status === 'starting' || r.status === 'awaiting_permission' ? 'working' : r.status !== 'closed' && (r.background_tasks?.length ?? 0) > 0 ? 'running' : 'idle',
    cwd: r.cwd,
    provider_session_id: r.native_thread_id ?? null,
    connection_id: null,
    created_by: r.created_by,
    created_at: r.created_at,
    last_active_at: r.last_active_at,
    archived: r.status === 'closed',
    // `parent_session_id` nests a linked thread under its group's root in
    // the sidebar (the delegated-worker rule, `delegatedParentId`).
    meta: {
      ...kept,
      surface: 'threads',
      runtime: 'structured',
      ...(r.parent_id ? { parent_session_id: r.parent_id } : {}),
      // The row's own attention (lib/status.ts `sessionState` reads these):
      // a pending request is "needs you" after a reload too, and a failed /
      // stopped last turn never reads idle.
      ...(r.status === 'awaiting_permission' ? { runtime_waiting: true } : {}),
      ...(r.last_turn_status ? { last_turn_status: r.last_turn_status } : {}),
      ...(r.last_error_class ? { last_error_class: r.last_error_class } : {}),
    },
  };
  threadSessionRowCache.set(r, mapped);
  return mapped;
}

/** `RuntimeSession.last_turn_status` / `last_error_class` from a
 *  `turn.terminal` event (the daemon's own classes: `usage_limit`, `auth`,
 *  else `provider`). */
export function turnOutcomeOf(
  status: string,
  failure?: { class?: string; message?: string } | null,
): Pick<RuntimeSession, 'last_turn_status' | 'last_error_class'> {
  if (status === 'failed') {
    const cls = failure?.class ?? '';
    const msg = failure?.message ?? '';
    const errorClass =
      cls === 'usage_limit' || /usage limit|rate.?limit|quota|\b429\b|too many requests/i.test(msg)
        ? 'usage_limit'
        : cls === 'auth' || /not (?:logged|signed) in|\/login|unauthori[sz]ed|\b401\b|invalid api key/i.test(msg)
          ? 'auth'
          : 'provider';
    return { last_turn_status: 'failed', last_error_class: errorClass };
  }
  if (status === 'interrupted' || status === 'cancelled') return { last_turn_status: 'interrupted', last_error_class: null };
  return { last_turn_status: 'completed', last_error_class: null };
}

/** A re-read row from a daemon without `last_turn_status` (the field is
 *  missing, not null) keeps the outcome this tab saw live; a newer daemon's
 *  own value always wins. */
export function keepOutcome(next: RuntimeSession, prev: RuntimeSession | undefined): RuntimeSession {
  if ('last_turn_status' in next || !prev?.last_turn_status) return next;
  return { ...next, last_turn_status: prev.last_turn_status, last_error_class: prev.last_error_class ?? null };
}

/** A Threads row backed by a runtime session (see `asThreadSession`). */
export function isRuntimeThread(s: Session | null | undefined): boolean {
  return s?.meta?.runtime === 'structured';
}

class RuntimeThreads {
  /** Every loaded runtime session, by id. */
  byId = $state<Record<string, RuntimeSession>>({});
  /** Threads in Plan mode: every prompt asks for a plan until the person
   *  leaves it (the daemon's `plan` is per prompt). This window only. */
  planIds = $state<Record<string, boolean>>({});
  /** Workspaces whose list has loaded once. */
  loaded = $state<Record<string, boolean>>({});
  loadErrors = $state<Record<string, string>>({});
  /** Threads that finished a turn while nobody was looking (the sidebar's dot). */
  unread = $state<Record<string, true>>({});
  /** Threads shown by an embed outside Threads (a turn ending there is seen). */
  private embedded = new Map<string, number>();
  /** Original composer attachments keyed by `queued_id` so Edit preserves filenames even before server hydration. */
  private queuedAttachments = new Map<string, { name: string; mime: string; data_base64: string }[]>();
  /** The thread each cached `queued_id` belongs to (to release them per thread). */
  private queuedOwner = new Map<string, string>();
  private seqListeners = new Set<(id: string, seq: number | null | undefined) => void>();
  private liveRefs = 0;
  private stopLive: (() => void) | null = null;

  onSequence(fn: (id: string, seq: number | null | undefined) => void): () => void {
    this.seqListeners.add(fn);
    return () => this.seqListeners.delete(fn);
  }

  private emitSequence(id: string, seq: number | null | undefined): void {
    for (const fn of this.seqListeners) fn(id, seq);
  }

  isActiveOrWaiting(id: string): boolean {
    const s = this.byId[id];
    if (!s || s.status === 'closed') return false;
    return (
      s.status === 'working' ||
      s.status === 'starting' ||
      s.status === 'awaiting_permission' ||
      (s.background_tasks?.length ?? 0) > 0
    );
  }

  get(id: string | null | undefined): RuntimeSession | null {
    return id ? (this.byId[id] ?? null) : null;
  }

  /** Live (or, with `closed`, closed) sessions, newest activity first. */
  live(closed = false): RuntimeSession[] {
    return Object.values(this.byId)
      .filter((s) => (s.status === 'closed') === closed)
      .sort((a, b) => b.last_active_at.localeCompare(a.last_active_at));
  }

  /** The live (or closed) sessions as Threads rows. */
  rows(closed = false): Session[] {
    return this.live(closed).map(asThreadSession);
  }

  /** (Re)load a workspace's sessions. `loaded` is set even on failure, so
   *  a failed list never holds Threads' pane bookkeeping back. */
  async load(workspaceId: string): Promise<void> {
    try {
      const rows = await api.get<RuntimeSession[]>(`/workspaces/${workspaceId}/runtime-sessions`);
      const next = { ...this.byId };
      for (const id of Object.keys(next)) if (next[id].workspace_id === workspaceId) delete next[id];
      for (const r of rows) {
        next[r.id] = keepOutcome(r, this.byId[r.id]);
        noteRuntimeWaiting(r.id, r.status === 'awaiting_permission', r.title);
        this.emitSequence(r.id, r.last_sequence);
      }
      this.byId = next;
      delete this.loadErrors[workspaceId];
    } catch (error) {
      this.loadErrors[workspaceId] = loadErrorText(error);
      throw error;
    } finally {
      this.loaded = { ...this.loaded, [workspaceId]: true };
    }
  }

  /** Add the caller's threads of EVERY workspace (`GET /runtime-sessions`):
   *  a thread started on another device lives in whatever workspace that
   *  device had open, and must still show here. Merges — the per-workspace
   *  {@link load} stays the authority for its own workspace. */
  async loadAll(): Promise<void> {
    const rows = await api.get<RuntimeSession[]>('/runtime-sessions');
    const next = { ...this.byId };
    for (const r of rows) {
      next[r.id] = keepOutcome(r, this.byId[r.id]);
      this.emitSequence(r.id, r.last_sequence);
    }
    // Every thread of the caller: the waiting set becomes exactly the rows
    // that wait (a thread deleted or answered elsewhere leaves it).
    syncRuntimeWaiting(rows.filter((r) => r.status === 'awaiting_permission').map((r) => ({ id: r.id, title: r.title })));
    this.byId = next;
  }

  /** Read one thread's row: an embed needs only its own, never the
   *  workspace's whole list. */
  async loadOne(id: string): Promise<RuntimeSession> {
    const r = keepOutcome(await api.get<RuntimeSession>(`/runtime-sessions/${id}`), this.byId[id]);
    this.byId[r.id] = r;
    noteRuntimeWaiting(r.id, r.status === 'awaiting_permission', r.title);
    this.emitSequence(r.id, r.last_sequence);
    return r;
  }

  /** Fetch the rows of `id`'s linked group not held yet (its chips). */
  async loadLinked(id: string): Promise<void> {
    const peers = await api.get<PeerInfo[]>(`/runtime-sessions/${id}/peers`);
    for (const peer of peers) if (!this.byId[peer.id]) this.adopt(peer.id);
  }

  /** An embed shows `id` while mounted (its finished turns are seen); returns the release. */
  watch(id: string): () => void {
    this.embedded.set(id, (this.embedded.get(id) ?? 0) + 1);
    this.markRead(id);
    return () => {
      const n = (this.embedded.get(id) ?? 1) - 1;
      if (n > 0) this.embedded.set(id, n);
      else this.embedded.delete(id);
    };
  }

  markRead(id: string): void {
    if (!this.unread[id]) return;
    const { [id]: _read, ...rest } = this.unread;
    this.unread = rest;
  }

  private onScreen(id: string): boolean {
    return this.embedded.has(id) || (router.module === 'threads' && threads.focusedSessionId === id);
  }

  /** Listen while Threads is mounted; returns the unsubscribe. */
  mount(): () => void {
    if (this.liveRefs++ === 0) {
      const offEvent = onLive(['agent_runtime_event'], (ev) => this.onEvent(ev.event as RuntimeEvent));
      // Events were lost (socket reconnect, lag, a daemon restart): re-read
      // the rows, or one stays "working" until Threads is re-entered.
      const offResync = appLive.onResync(() => void this.resync());
      this.stopLive = () => {
        offEvent();
        offResync();
      };
    }
    return () => {
      if (--this.liveRefs === 0) {
        this.stopLive?.();
        this.stopLive = null;
      }
    };
  }

  /** Re-read every loaded workspace's rows and the cross-workspace list. */
  async resync(): Promise<void> {
    const loaded = Object.keys(this.loaded).filter((w) => this.loaded[w]);
    await Promise.allSettled([...loaded.map((w) => this.load(w)), this.loadAll()]);
  }

  /** Release cached attachments of `id`'s queued messages that left its
   *  queue (the daemon delivered them), or all of them (`keep` null). */
  private releaseQueued(id: string, keep: readonly QueuedPrompt[] | null): void {
    const still = new Set((keep ?? []).map((q) => q.id));
    for (const [qid, owner] of this.queuedOwner) {
      if (owner !== id || still.has(qid)) continue;
      this.queuedOwner.delete(qid);
      this.queuedAttachments.delete(qid);
    }
  }

  /** Ids being fetched because an event named a thread we don't hold yet. */
  private fetching = new Set<string>();

  /** A thread an AGENT opened (`otto.thread_open`) is first seen through its
   *  events: fetch the row once so it appears in the sidebar and the group. */
  private adopt(id: string): void {
    if (this.fetching.has(id)) return;
    this.fetching.add(id);
    void api
      .get<RuntimeSession>(`/runtime-sessions/${id}`)
      .then((r) => {
        this.put(r);
        noteRuntimeWaiting(r.id, r.status === 'awaiting_permission', r.title);
        this.emitSequence(r.id, r.last_sequence);
      })
      .catch(() => {
        /* not ours to see (another user's thread) */
      })
      .finally(() => this.fetching.delete(id));
  }

  /** `id`'s linked group — its root first, then the threads opened in it
   *  (oldest first). Just `[id]` for an unlinked thread. */
  group(id: string | null | undefined): RuntimeSession[] {
    const s = this.get(id);
    if (!s) return [];
    const root = s.parent_id ?? s.id;
    return Object.values(this.byId)
      .filter((r) => r.status !== 'closed' && (r.id === root || r.parent_id === root))
      .sort((a, b) => (a.id === root ? -1 : b.id === root ? 1 : a.created_at.localeCompare(b.created_at)));
  }

  /** The newest status an event carried per thread, kept even while its row
   *  is not here yet: a fast turn can end before the HTTP response that
   *  creates the row lands (a council's judge), and that response must not
   *  bring back a stale "working" (`put`). One small entry per thread. */
  private seen: Record<string, { status: RuntimeSessionStatus; ts: string; outcome?: Pick<RuntimeSession, 'last_turn_status' | 'last_error_class'> }> = {};

  /** Insert a row from an HTTP response, keeping a newer live status. */
  put(r: RuntimeSession): void {
    const s = this.seen[r.id];
    this.byId[r.id] = s && Date.parse(s.ts) > Date.parse(r.last_active_at) ? { ...r, status: s.status, ...s.outcome } : r;
  }

  private onEvent(ev: RuntimeEvent): void {
    if (ev.type === 'provider_session.updated') this.seen[ev.session_id] = { status: SESSION_STATUS[ev.provider_session.status], ts: ev.ts, outcome: this.seen[ev.session_id]?.outcome };
    else if (ev.type === 'turn.terminal' && this.seen[ev.session_id]) this.seen[ev.session_id].outcome = turnOutcomeOf(ev.status, ev.failure);
    if (!this.byId[ev.session_id]) this.adopt(ev.session_id);
    if (ev.type === 'queue.updated') this.releaseQueued(ev.session_id, ev.queued_prompts ?? []);
    // A model / access switch was recorded — the agent's own too (ACP
    // `current_mode_update`): the row, and so the chips, re-read it.
    // (`selection:` notices — the shape settingsChange.ts reads.)
    if (ev.type === 'turn_item.updated' && ev.turn_item.type === 'system_notice' && ev.turn_item.id.startsWith('selection:')) this.adopt(ev.session_id);
    // A turn that ended (done or failed) off screen leaves an unread dot.
    if (ev.type === 'turn.terminal' && (ev.status === 'completed' || ev.status === 'failed') && !this.onScreen(ev.session_id)) {
      this.unread = { ...this.unread, [ev.session_id]: true };
    }
    // The row follows status / turn changes only — not every streamed chunk.
    const row = this.byId[ev.session_id];
    if (row && row.status !== 'closed' && (ev.type === 'provider_session.updated' || ev.type === 'provider_turn.updated')) {
      if (ev.type === 'provider_session.updated') {
        const ps = ev.provider_session;
        const status = SESSION_STATUS[ps.status];
        // A new turn clears how the last one ended (the daemon's rule for `last_turn_status`).
        const fresh = status === 'working' || status === 'starting' ? { last_turn_status: null, last_error_class: null } : {};
        // A provider switch moves the row (title glyph, details card) to the new provider.
        const moved = ps.provider ? providerPatch(row, ps.provider, ps.provider_slug, ps.model) : {};
        this.byId[ev.session_id] = { ...row, ...fresh, ...moved, status, background_tasks: ps.background_tasks ?? [], capabilities: ps.capabilities ?? row.capabilities, title: ps.title ?? row.title, last_active_at: ev.ts };
      } else this.byId[ev.session_id] = { ...row, last_active_at: ev.ts };
    }
    // How the turn ended stays on the row until the next one starts — live,
    // so an older daemon (no `last_turn_status` on the row) still shows it
    // until a reload.
    if (row && row.status !== 'closed' && ev.type === 'turn.terminal') {
      const cur = this.byId[ev.session_id] ?? row;
      this.byId[ev.session_id] = { ...cur, ...turnOutcomeOf(ev.status, ev.failure) };
    }
  }

  /** Create a session (the CLI is spawned before this resolves) and send
   *  the opening prompt, if any. */
  async start(
    workspaceId: string,
    req: {
      provider: RuntimeProviderOrSlug;
      cwd: string | null;
      title: string;
      /** `title` is a placeholder the agent's own name for it replaces. */
      titleAuto?: boolean;
      model: string | null;
      effort?: string;
      /** The opening turn in plan mode (where the agent has one). */
      plan?: boolean;
      prompt: string;
      /** Images / PDFs for the first message. */
      attachments?: readonly { name: string; mime: string; data_base64: string }[];
      /** The thread's access mode from the first turn on. Left out, a new
       *  thread starts in the daemon's default — Full access while Settings ›
       *  Providers skips permission prompts (the same rule terminal sessions
       *  follow), else Approval required. Embeds always name one. */
      accessMode?: RuntimeAccessMode;
      /** An embedded thread's tag and what it is about (`meta.source` /
       *  `meta.origin`, docs/features/threads-structured-runtime.md). */
      source?: EmbeddedThreadSource;
      origin?: EmbeddedOrigin;
    },
  ): Promise<RuntimeSession> {
    const path = `/workspaces/${workspaceId}/runtime-sessions`;
    const body = {
      provider: req.provider,
      cwd: req.cwd,
      title: req.title,
      ...(req.titleAuto ? { title_auto: true } : {}),
      model: req.model,
      ...(req.accessMode ? { access_mode: req.accessMode } : {}),
    };
    // The thread keeps its effort for later turns (the first prompt still
    // names it), and an embed its tag.
    const full = {
      ...body,
      ...(req.effort ? { effort: req.effort } : {}),
      ...(req.source ? { source: req.source, ...(req.origin ? { origin: req.origin } : {}) } : {}),
    };
    let created: RuntimeSession;
    try {
      created = await api.post<RuntimeSession>(path, full);
    } catch (e) {
      // An older daemon refuses the newer fields (400 / 422): a plain thread
      // still works — the host remembers its id, the prompt carries the effort.
      const status = (e as { status?: unknown } | null)?.status;
      const extra = !!req.source || !!req.effort;
      const refused = extra && (status === 400 || status === 422) && /source|origin|effort|unknown field/i.test(String((e as Error)?.message ?? ''));
      if (!refused) throw e;
      created = await api.post<RuntimeSession>(path, body);
    }
    this.put(created);
    if (req.plan) this.planIds[created.id] = true;
    this.emitSequence(created.id, created.last_sequence);
    const pics = (req.attachments ?? []).filter((f) => f.mime.startsWith('image/'));
    const docs = (req.attachments ?? []).filter((f) => !f.mime.startsWith('image/'));
    if (req.prompt || pics.length || docs.length) {
      await this.prompt(
        created.id,
        {
          text: req.prompt,
          ...(req.effort ? { effort: req.effort } : {}),
          ...(pics.length
            ? { images: pics.map(({ name, mime, data_base64 }) => ({ ...(name ? { name } : {}), mime, data_base64 })) }
            : {}),
          ...(docs.length ? { files: docs.map((d) => ({ ...d })) } : {}),
        },
        req.attachments,
      );
    }
    return created;
  }

  /** The embedded thread a host started for `originId` (a PR, a connection):
   *  `GET …/runtime-sessions?source=&origin_id=`, never a linked thread its
   *  agent opened (those inherit the origin), an OPEN one before a closed
   *  one, then the newest activity. `rememberedId` is the id the host kept
   *  for a daemon that stores no origin. The pick lands in `byId`. */
  async findByOrigin(
    workspaceId: string,
    source: string,
    originId: string,
    rememberedId: string | null = null,
  ): Promise<RuntimeSession | null> {
    const q = `source=${encodeURIComponent(source)}&origin_id=${encodeURIComponent(originId)}`;
    const rows = await api.get<RuntimeSession[]>(`/workspaces/${workspaceId}/runtime-sessions?${q}`);
    const originOf = (r: RuntimeSession): string | null => {
      const o = r.meta?.origin as { id?: unknown } | undefined;
      return typeof o?.id === 'string' ? o.id : null;
    };
    const mine = (Array.isArray(rows) ? rows : []).filter(
      (r) => !r.parent_id && (r.id === rememberedId || (r.meta?.source === source && originOf(r) === originId)),
    );
    const closed = (r: RuntimeSession): number => (r.status === 'closed' ? 1 : 0);
    mine.sort((a, b) => closed(a) - closed(b) || b.last_active_at.localeCompare(a.last_active_at));
    const hit = mine[0] ?? null;
    if (hit) this.byId[hit.id] = hit;
    return hit;
  }

  /** "Delegate to…" (or several): new threads linked to `parentId`,
   *  each sent `req.prompt`; all or none. Their rows land before this resolves. */
  async askChildren(parentId: string, req: AskChildReq): Promise<RuntimeSession[]> {
    const { children } = await api.post<AskChildResp>(`/runtime-sessions/${parentId}/children`, req);
    for (const c of children) {
      // A fast subagent's turn can end before this answer lands: keep the
      // newer live status its events already brought (`put`), never a stale
      // "working" that would keep "Waiting on …" up until a reload.
      this.put(c);
      this.emitSequence(c.id, c.last_sequence);
    }
    return children;
  }

  async prompt(id: string, req: RuntimePromptReq, localAttachments?: readonly QueuedAttachment[]): Promise<RuntimePromptResp> {
    if (this.planIds[id] && req.plan === undefined) req = { ...req, plan: true };
    const resp = await api.post<RuntimePromptResp>(`/runtime-sessions/${id}/prompt`, req);
    if (resp.queued_id) {
      const cached: QueuedAttachment[] = localAttachments?.length
        ? localAttachments.map((a) => ({ ...a }))
        : [
            ...(req.images ?? []).map((img, i) => ({
              name: img.name?.trim() || ((req.images?.length ?? 0) > 1 ? `Screenshot ${i + 1}` : 'Screenshot'),
              mime: img.mime,
              data_base64: img.data_base64,
            })),
            ...(req.files ?? []).map((f) => ({ ...f })),
          ];
      if (cached.length > 0) {
        if (this.queuedAttachments.size >= 64) {
          const oldest = this.queuedAttachments.keys().next().value;
          if (oldest) {
            this.queuedAttachments.delete(oldest);
            this.queuedOwner.delete(oldest);
          }
        }
        this.queuedAttachments.set(resp.queued_id, cached);
        this.queuedOwner.set(resp.queued_id, id);
      }
    }
    this.emitSequence(id, resp.sequence);
    let row = this.byId[id];
    if (row) {
      let changed = false;
      if (req.access_mode && row.access_mode !== req.access_mode) {
        row = { ...row, access_mode: req.access_mode };
        changed = true;
      }
      if (req.model !== undefined) {
        const nextModel = req.model.trim() || undefined;
        if (row.model !== nextModel) {
          row = { ...row, model: nextModel };
          changed = true;
        }
      }
      if (changed) this.byId[id] = row;
    }
    return resp;
  }

  /** Stop the running turn. `interrupted: false` = no turn ran (background
   *  work, if any, keeps running — `background_tasks` counts it). */
  async interrupt(id: string): Promise<RuntimeInterruptResp | undefined> {
    return api.post<RuntimeInterruptResp | undefined>(`/runtime-sessions/${id}/interrupt`);
  }

  async respond(id: string, requestId: string, decision: ApprovalDecision): Promise<void> {
    await api.post(`/runtime-sessions/${id}/requests/${requestId}`, { decision });
  }

  async respondQuestion(id: string, requestId: string, answers: QuestionAnswer[]): Promise<void> {
    await api.post(`/runtime-sessions/${id}/requests/${encodeURIComponent(requestId)}`, { answers });
  }

  async dismissQuestion(id: string, requestId: string, action: 'decline' | 'cancel'): Promise<void> {
    await api.post(`/runtime-sessions/${id}/requests/${encodeURIComponent(requestId)}`, { question_action: action });
  }

  async rename(id: string, title: string): Promise<void> {
    this.byId[id] = await api.patch<RuntimeSession>(`/runtime-sessions/${id}`, { title });
  }

  async close(id: string): Promise<void> {
    this.byId[id] = await api.post<RuntimeSession>(`/runtime-sessions/${id}/close`);
  }

  /** Revive a closed or dormant thread; a live agent is preserved. */
  async reopen(id: string): Promise<RuntimeSession> {
    const r = await api.post<RuntimeSession>(`/runtime-sessions/${id}/reopen`);
    this.byId[id] = r;
    this.emitSequence(id, r.last_sequence);
    return r;
  }

  /** Stop the agent and remove the thread with its history. */
  async remove(id: string): Promise<void> {
    await api.del(`/runtime-sessions/${id}`);
    runtimeDrafts.forget(id);
    this.releaseQueued(id, null);
    this.markRead(id);
    const next = { ...this.byId };
    delete next[id];
    this.byId = next;
  }

  /** Drop a queued message and return its text + attachments so Edit can restore them. */
  async unqueue(
    id: string,
    queuedId: string,
  ): Promise<{ text?: string; attachments: { name: string; mime: string; data_base64: string }[] }> {
    const cached = this.queuedAttachments.get(queuedId);
    const resp = await api.del<UnqueuePromptResp | void>(`/runtime-sessions/${id}/queue/${queuedId}`);
    this.queuedAttachments.delete(queuedId);
    this.queuedOwner.delete(queuedId);
    if (cached && cached.length > 0) {
      return { text: resp && typeof resp === 'object' ? resp.text : undefined, attachments: cached };
    }
    if (resp && typeof resp === 'object') {
      const imgs = resp.images ?? [];
      const docs = resp.files ?? [];
      const reconstructed = [
        ...imgs.map((img, idx) => ({
          name: img.name?.trim() || (imgs.length > 1 ? `Screenshot ${idx + 1}` : 'Screenshot'),
          mime: img.mime,
          data_base64: img.data_base64,
        })),
        ...docs.map((f) => ({ name: f.name || 'Attachment', mime: f.mime, data_base64: f.data_base64 })),
      ];
      return { text: resp.text, attachments: reconstructed };
    }
    return { attachments: [] };
  }

  /** Deliver a queued message now (steer it in, or stop the turn and send it), or retry a failed one (`queue`). */
  async deliverQueued(id: string, queuedId: string, delivery: PromptDelivery): Promise<RuntimePromptResp> {
    // A retry (`queue`) keeps the message queued: its attachments stay cached
    // for Edit until the daemon delivers it (`queue.updated` releases them).
    if (delivery !== 'queue') {
      this.queuedAttachments.delete(queuedId);
      this.queuedOwner.delete(queuedId);
    }
    return api.post<RuntimePromptResp>(`/runtime-sessions/${id}/queue/${queuedId}`, { delivery });
  }
}

// Install persistence when the runtime surface loads (Threads and embedded chats).
// Its dependency graph ends at the API/draft store, never back at this module.
installRuntimeDraftRecovery();
export const runtimeThreads = new RuntimeThreads();

// Agent UI control (otto.ui_* — the agent drives Otto beside the thread): a
// thread's grant lives on its runtime row, so tell the UI-control store
// where to read and patch it, and how to bring the thread up.
uiControl.addSource({
  get: (id) => {
    const r = runtimeThreads.get(id);
    return r
      ? { id: r.id, provider: sessionProviderSlug(r), title: r.title, meta: r.ui_control ? { ui_control: r.ui_control } : {} }
      : null;
  },
  patch: (id, grant) => {
    const r = runtimeThreads.byId[id];
    if (r) runtimeThreads.byId[id] = { ...r, ui_control: grant };
  },
  onScreen: (id) => router.module === 'threads' && threads.paneIds.includes(id),
  reveal: (id) => {
    if (router.module !== 'threads') router.go(`threads/${id}`);
    threads.selectThread(id);
  },
});


/** The row change a `provider_session.updated` carrying `provider` makes:
 *  `{}` when the thread still runs there, else its new provider, account
 *  slug (`meta.provider_slug`, kept only for a named account) and model. */
function providerPatch(
  row: Pick<RuntimeSession, 'provider' | 'meta' | 'model'>,
  provider: RuntimeProvider,
  slug: string | undefined,
  model: string | null,
): Partial<Pick<RuntimeSession, 'provider' | 'meta' | 'model'>> {
  const to = slug || provider;
  if (row.provider === provider && sessionProviderSlug(row) === to) return {};
  const meta: Record<string, unknown> = { ...(row.meta ?? {}) };
  if (to !== provider) meta.provider_slug = to;
  else delete meta.provider_slug;
  return { provider, meta, model: model ?? undefined };
}
