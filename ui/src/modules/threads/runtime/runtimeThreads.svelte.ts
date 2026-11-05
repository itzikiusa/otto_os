// Structured runtime sessions in Threads (docs/features/threads-structured-runtime.md):
// the session rows per workspace (status kept live from `agent_runtime_event`)
// and the session actions. An open thread's conversation is runtimeView.svelte.ts,
// loaded with the view.

import { api } from '../../../lib/api/client';
import { onLive } from '../../../lib/live';
import { router } from '../../../lib/router.svelte';
import { noteRuntimeWaiting } from '../../../lib/stores/runtimeAttention.svelte';
import { uiControl } from '../../../lib/stores/uiControl.svelte';
import type {
  ApprovalDecision,
  PromptDelivery,
  ProviderSessionStatus,
  RuntimeEvent,
  RuntimePromptReq,
  RuntimePromptResp,
  RuntimeProvider,
  RuntimeSession,
  RuntimeSessionStatus,
  Session,
} from '../../../lib/api/types';
import { threads } from '../threadsStore.svelte';

const RUNTIME_PROVIDERS: readonly string[] = ['claude', 'codex', 'gemini', 'grok', 'opencode', 'pi'];

/** Providers a new thread runs on the structured runtime (the rest stay PTY). */
export function isRuntimeProvider(provider: string): provider is RuntimeProvider {
  return RUNTIME_PROVIDERS.includes(provider);
}

/** `set_session_status` in otto-agent-protocol's runtime.rs. */
const SESSION_STATUS: Record<ProviderSessionStatus, RuntimeSessionStatus> = {
  starting: 'starting',
  ready: 'idle',
  running: 'working',
  waiting: 'awaiting_permission',
  stopped: 'dormant',
  error: 'dormant',
};

/** A runtime session as the Threads sidebar's row shape (never sent to the
 *  PTY session routes — `meta.runtime` marks it). */
export function asThreadSession(r: RuntimeSession): Session {
  return {
    id: r.id,
    workspace_id: r.workspace_id,
    kind: 'agent',
    provider: r.provider,
    title: r.title,
    // Background work the agent started keeps the row working between turns.
    status: r.status === 'working' || r.status === 'starting' || (r.status !== 'closed' && (r.background_tasks?.length ?? 0) > 0) ? 'working' : 'idle',
    cwd: r.cwd,
    provider_session_id: r.native_thread_id ?? null,
    connection_id: null,
    created_by: r.created_by,
    created_at: r.created_at,
    last_active_at: r.last_active_at,
    archived: r.status === 'closed',
    // `parent_session_id` nests a linked thread under its group's root in
    // the sidebar (the delegated-worker rule, `delegatedParentId`).
    meta: { surface: 'threads', runtime: 'structured', ...(r.parent_id ? { parent_session_id: r.parent_id } : {}) },
  };
}

/** A Threads row backed by a runtime session (see `asThreadSession`). */
export function isRuntimeThread(s: Session | null | undefined): boolean {
  return s?.meta?.runtime === 'structured';
}

class RuntimeThreads {
  /** Every loaded runtime session, by id. */
  byId = $state<Record<string, RuntimeSession>>({});
  /** Workspaces whose list has loaded once. */
  loaded = $state<Record<string, boolean>>({});
  private liveRefs = 0;
  private stopLive: (() => void) | null = null;

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
        next[r.id] = r;
        noteRuntimeWaiting(r.id, r.status === 'awaiting_permission');
      }
      this.byId = next;
    } finally {
      this.loaded = { ...this.loaded, [workspaceId]: true };
    }
  }

  /** Listen while Threads is mounted; returns the unsubscribe. */
  mount(): () => void {
    if (this.liveRefs++ === 0) this.stopLive = onLive(['agent_runtime_event'], (ev) => this.onEvent(ev.event as RuntimeEvent));
    return () => {
      if (--this.liveRefs === 0) {
        this.stopLive?.();
        this.stopLive = null;
      }
    };
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
        this.byId[r.id] = r;
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

  private onEvent(ev: RuntimeEvent): void {
    if (!this.byId[ev.session_id]) this.adopt(ev.session_id);
    // The row follows status / turn changes only — not every streamed chunk.
    const row = this.byId[ev.session_id];
    if (row && row.status !== 'closed' && (ev.type === 'provider_session.updated' || ev.type === 'provider_turn.updated')) {
      if (ev.type === 'provider_session.updated') {
        const ps = ev.provider_session;
        this.byId[ev.session_id] = { ...row, status: SESSION_STATUS[ps.status], background_tasks: ps.background_tasks ?? [], last_active_at: ev.ts };
      } else this.byId[ev.session_id] = { ...row, last_active_at: ev.ts };
    }
  }

  /** Create a session (the CLI is spawned before this resolves) and send
   *  the opening prompt, if any. */
  async start(
    workspaceId: string,
    req: { provider: RuntimeProvider; cwd: string | null; title: string; model: string | null; effort?: string; prompt: string },
  ): Promise<RuntimeSession> {
    const created = await api.post<RuntimeSession>(`/workspaces/${workspaceId}/runtime-sessions`, {
      provider: req.provider,
      cwd: req.cwd,
      title: req.title,
      model: req.model,
      // No access mode: a new thread starts in the daemon's default — Full
      // access while Settings › Providers skips permission prompts (the same
      // rule terminal sessions follow), else Approval required.
    });
    this.byId[created.id] = created;
    if (req.prompt) await this.prompt(created.id, { text: req.prompt, ...(req.effort ? { effort: req.effort } : {}) });
    return created;
  }

  async prompt(id: string, req: RuntimePromptReq): Promise<RuntimePromptResp> {
    const resp = await api.post<RuntimePromptResp>(`/runtime-sessions/${id}/prompt`, req);
    const mode = req.access_mode;
    if (mode) {
      // Sticky on the session (new threads start in the daemon default).
      const row = this.byId[id];
      if (row && row.access_mode !== mode) this.byId[id] = { ...row, access_mode: mode };
    }
    return resp;
  }

  async interrupt(id: string): Promise<void> {
    await api.post(`/runtime-sessions/${id}/interrupt`);
  }

  async respond(id: string, requestId: string, decision: ApprovalDecision): Promise<void> {
    await api.post(`/runtime-sessions/${id}/requests/${requestId}`, { decision });
  }

  async rename(id: string, title: string): Promise<void> {
    this.byId[id] = await api.patch<RuntimeSession>(`/runtime-sessions/${id}`, { title });
  }

  async close(id: string): Promise<void> {
    this.byId[id] = await api.post<RuntimeSession>(`/runtime-sessions/${id}/close`);
  }

  /** A closed thread takes messages again (its agent resumes). */
  async reopen(id: string): Promise<RuntimeSession> {
    const r = await api.post<RuntimeSession>(`/runtime-sessions/${id}/reopen`);
    this.byId[id] = r;
    return r;
  }

  /** Stop the agent and remove the thread with its history. */
  async remove(id: string): Promise<void> {
    await api.del(`/runtime-sessions/${id}`);
    const next = { ...this.byId };
    delete next[id];
    this.byId = next;
  }

  /** Drop a queued message. */
  async unqueue(id: string, queuedId: string): Promise<void> {
    await api.del(`/runtime-sessions/${id}/queue/${queuedId}`);
  }

  /** Deliver a queued message now: steer it in, or stop the turn and send it. */
  async deliverQueued(id: string, queuedId: string, delivery: PromptDelivery): Promise<RuntimePromptResp> {
    return api.post<RuntimePromptResp>(`/runtime-sessions/${id}/queue/${queuedId}`, { delivery });
  }
}

export const runtimeThreads = new RuntimeThreads();

// Agent UI control (otto.ui_* — the agent drives Otto beside the thread): a
// thread's grant lives on its runtime row, so tell the UI-control store
// where to read and patch it, and how to bring the thread up.
uiControl.addSource({
  get: (id) => {
    const r = runtimeThreads.get(id);
    return r ? { id: r.id, provider: r.provider, title: r.title, meta: r.ui_control ? { ui_control: r.ui_control } : {} } : null;
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

