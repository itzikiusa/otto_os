// Structured runtime sessions in Threads (docs/features/threads-structured-runtime.md):
// the session rows per workspace (status kept live from `agent_runtime_event`)
// and the session actions. An open thread's conversation is runtimeView.svelte.ts,
// loaded with the view.

import { api } from '../../../lib/api/client';
import { onLive } from '../../../lib/live';
import { noteRuntimeSession } from '../../../lib/stores/runtimeAttention.svelte';
import type {
  ApprovalDecision,
  ProviderSessionStatus,
  RuntimeEvent,
  RuntimePromptResp,
  RuntimeProvider,
  RuntimeSession,
  RuntimeSessionStatus,
  Session,
} from '../../../lib/api/types';

const RUNTIME_PROVIDERS: readonly string[] = ['claude', 'codex', 'gemini', 'grok'];

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
    status: r.status === 'working' || r.status === 'starting' ? 'working' : 'idle',
    cwd: r.cwd,
    provider_session_id: r.native_thread_id ?? null,
    connection_id: null,
    created_by: r.created_by,
    created_at: r.created_at,
    last_active_at: r.last_active_at,
    archived: r.status === 'closed',
    meta: { surface: 'threads', runtime: 'structured' },
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
        noteRuntimeSession(r.id);
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

  private onEvent(ev: RuntimeEvent): void {
    // The row follows status / turn changes only — not every streamed chunk.
    const row = this.byId[ev.session_id];
    if (row && row.status !== 'closed' && (ev.type === 'provider_session.updated' || ev.type === 'provider_turn.updated')) {
      const status = ev.type === 'provider_session.updated' ? SESSION_STATUS[ev.provider_session.status] : row.status;
      this.byId[ev.session_id] = { ...row, status, last_active_at: ev.ts };
    }
  }

  /** Create a session (the CLI is spawned before this resolves) and send
   *  the opening prompt, if any. */
  async start(
    workspaceId: string,
    req: { provider: RuntimeProvider; cwd: string | null; title: string; model: string | null; prompt: string },
  ): Promise<RuntimeSession> {
    const created = await api.post<RuntimeSession>(`/workspaces/${workspaceId}/runtime-sessions`, {
      provider: req.provider,
      cwd: req.cwd,
      title: req.title,
      model: req.model,
    });
    this.byId[created.id] = created;
    if (req.prompt) await this.prompt(created.id, req.prompt);
    return created;
  }

  prompt(id: string, text: string): Promise<RuntimePromptResp> {
    return api.post<RuntimePromptResp>(`/runtime-sessions/${id}/prompt`, { text });
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
}

export const runtimeThreads = new RuntimeThreads();
