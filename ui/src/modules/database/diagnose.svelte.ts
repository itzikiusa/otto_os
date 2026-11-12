// "Diagnose with an agent" — the stateful half (rules: diagnose-prompt.ts).
//
// Flow: an entry point (the ended-SSH panel, the DB connect-error banner, the
// connection form's failed Test) calls `request` → the consent dialog
// (DiagnoseAside) → `confirm` finds this connection's earlier diagnosis
// thread or creates one (source `connection-diagnose`, origin = the
// connection; an older daemon that rejects those fields gets a plain thread)
// in APPROVAL-REQUIRED mode — every command the agent runs waits for the
// user's OK in the embed — and sends the opening prompt. The thread then
// shows in the Connections page's side split; Close hides it and keeps the
// thread, the next Diagnose on the same connection shows it again.
//
// "Finished" is read from the thread's own row (GET /runtime-sessions/{id},
// polled while the panel shows a running diagnosis): idle again after the
// prompt's first event. This module never imports the Threads feature — the
// embed comes through lib/runtimeThreadPort.

import { api, ApiError } from '../../lib/api/client';
import { loadErrorText } from '../../lib/loadError';
import { pollWhileVisible, type Poller } from '../../lib/poll';
import { defaultAgentProvider } from '../../lib/providers';
import { ws } from '../../lib/stores/workspace.svelte';
import type { Connection, CreateRuntimeSessionReq, RuntimePromptResp, RuntimeSession } from '../../lib/api/types';
import { DIAGNOSE_SOURCE, diagnosePrompt, originLabel, pickDiagnoseThread, type DiagnoseProbe, type DiagnoseThreadRow } from './diagnose-prompt';

export type DiagnosePhase = 'starting' | 'running' | 'finished' | 'failed';

/** Providers the structured runtime runs (RuntimeProvider). */
const RUNTIME_PROVIDERS = new Set(['claude', 'codex', 'gemini', 'grok', 'agy', 'opencode', 'pi']);
const POLL_MS = 3000;

interface Active {
  conn: Connection;
  probe: DiagnoseProbe | null;
  workspaceId: string;
  sessionId: string | null;
  phase: DiagnosePhase;
  error: string | null;
  /** `sequence` of the prompt's first event — idle past it = finished. */
  startSeq: number;
}

class ConnDiagnose {
  /** The consent dialog, when one is up. */
  consent = $state<{ conn: Connection; probe: DiagnoseProbe | null } | null>(null);
  active = $state<Active | null>(null);
  /** The side panel is open (Close hides it; the thread stays). */
  shown = $state(false);
  maximized = $state(false);
  private poller: Poller | null = null;

  /** An entry point's "Diagnose with an agent…". The same connection's
   *  diagnosis that is already under way just shows again — no new consent. */
  request(conn: Connection, probe: DiagnoseProbe | null): void {
    if (this.active?.conn.id === conn.id && this.active.phase !== 'failed') {
      this.shown = true;
      this.watch();
      return;
    }
    this.consent = { conn, probe };
  }

  cancel(): void {
    this.consent = null;
  }

  /** Consent given: start (or reuse) the thread and send the prompt. */
  async confirm(): Promise<void> {
    const c = this.consent;
    this.consent = null;
    if (c) await this.start(c.conn, c.probe);
  }

  /** Retry after a failed start. */
  async retry(): Promise<void> {
    const a = this.active;
    if (a) await this.start(a.conn, a.probe);
  }

  close(): void {
    this.shown = false;
    this.maximized = false;
    this.unwatch();
  }

  private async start(conn: Connection, probe: DiagnoseProbe | null): Promise<void> {
    const workspaceId = ws.currentId ?? conn.workspace_id ?? '';
    this.active = { conn, probe, workspaceId, sessionId: null, phase: 'starting', error: null, startSeq: 0 };
    this.shown = true;
    try {
      if (!workspaceId) throw new Error('Pick a workspace first — the diagnosis thread belongs to one.');
      const session = (await this.existing(workspaceId, conn.id)) ?? (await this.create(workspaceId, conn));
      if (this.active?.conn.id !== conn.id) return;
      this.active = { ...this.active, sessionId: session.id };
      const resp = await api.post<RuntimePromptResp>(`/runtime-sessions/${session.id}/prompt`, {
        text: diagnosePrompt(conn, probe),
        access_mode: 'approval-required',
      });
      if (this.active?.conn.id !== conn.id) return;
      this.active = { ...this.active, phase: 'running', startSeq: resp.sequence ?? 0 };
      this.watch();
    } catch (e) {
      if (this.active?.conn.id === conn.id) this.active = { ...this.active, phase: 'failed', error: loadErrorText(e) };
    }
  }

  /** This connection's earlier diagnosis thread, if the daemon lists one. */
  private async existing(workspaceId: string, connId: string): Promise<DiagnoseThreadRow | null> {
    try {
      const q = `source=${encodeURIComponent(DIAGNOSE_SOURCE)}&origin_id=${encodeURIComponent(connId)}`;
      const rows = await api.get<DiagnoseThreadRow[]>(`/workspaces/${workspaceId}/runtime-sessions?${q}`);
      return pickDiagnoseThread(Array.isArray(rows) ? rows : [], connId);
    } catch {
      return null; // the lookup is a convenience: a new thread works too
    }
  }

  private async create(workspaceId: string, conn: Connection): Promise<RuntimeSession> {
    const def = defaultAgentProvider();
    const base: CreateRuntimeSessionReq = {
      provider: (RUNTIME_PROVIDERS.has(def) ? def : 'claude') as CreateRuntimeSessionReq['provider'],
      title: `Diagnose ${conn.name}`,
      // The most restrictive mode: every command waits for the user's OK.
      access_mode: 'approval-required',
    };
    const path = `/workspaces/${workspaceId}/runtime-sessions`;
    try {
      return await api.post<RuntimeSession>(path, {
        ...base,
        source: 'connection-diagnose',
        origin: { kind: 'connection', id: conn.id, label: originLabel(conn.name) },
      });
    } catch (e) {
      // An older daemon may refuse the unknown fields: a plain thread still works.
      if (e instanceof ApiError && (e.status === 400 || e.status === 422)) return api.post<RuntimeSession>(path, base);
      throw e;
    }
  }

  private watch(): void {
    if (this.poller || this.active?.phase !== 'running') return;
    this.poller = pollWhileVisible(() => this.poll(), { ms: POLL_MS, immediate: false });
  }

  private unwatch(): void {
    this.poller?.stop();
    this.poller = null;
  }

  private async poll(): Promise<void> {
    const a = this.active;
    if (!a?.sessionId || a.phase !== 'running' || !this.shown) return this.unwatch();
    try {
      const row = await api.bg.get<RuntimeSession>(`/runtime-sessions/${a.sessionId}`);
      const done = row.status === 'closed' || (row.status === 'idle' && row.last_sequence > a.startSeq);
      if (done && this.active?.sessionId === a.sessionId) {
        this.active = { ...this.active, phase: 'finished' };
        this.unwatch();
      }
    } catch {
      /* the next tick retries; the embed shows its own errors */
    }
  }
}

export const connDiagnose = new ConnDiagnose();
