// "Diagnose with an agent" — the stateful half (rules: diagnose-prompt.ts).
//
// Flow: an entry point (the ended-SSH panel, the DB connect-error banner, the
// connection form's failed Test) calls `request` → the side panel
// (DiagnoseAside) opens and this connection's earlier diagnosis thread is
// looked up through the Threads store (source `connection-diagnose`, origin
// = the connection; an open thread before a closed one).
//   - None yet: the panel is the Threads start composer (the embed's draft
//     state) under the consent text — the person picks the agent, model,
//     effort and access (approval-required unless they choose otherwise:
//     every command the agent runs waits for their OK) and starts it.
//   - One exists: it shows, and the consent dialog offers to run the checks
//     again in it — reopened when the daemon closed it, queued behind a turn
//     still running.
// Close hides the panel and keeps the thread; the next Diagnose on the same
// connection shows it again. "Finished" is the embed's own status
// (`onstatus`): idle again after it was seen working. Every send goes
// through lib/runtimeThreadPort (never the runtime routes), so the embed's
// chip and status stay true; this module never imports the Threads feature.

import { loadErrorText } from '../../lib/loadError';
import { loadIdeRuntimeSessionPort, type RuntimeEmbedStatus } from '../../lib/runtimeThreadPort';
import { ws } from '../../lib/stores/workspace.svelte';
import type { Connection, RuntimeSession } from '../../lib/api/types';
import { DIAGNOSE_SOURCE, diagnosePrompt, type DiagnoseProbe } from './diagnose-prompt';

/** looking: the earlier thread is being looked up · ready: nothing running
 *  (the start composer, or an idle thread) · running / finished: a diagnosis
 *  this panel started · failed: the lookup or a send failed. */
export type DiagnosePhase = 'looking' | 'ready' | 'running' | 'finished' | 'failed';

interface Active {
  conn: Connection;
  probe: DiagnoseProbe | null;
  workspaceId: string;
  /** Null: no thread yet (the draft composer). */
  sessionId: string | null;
  phase: DiagnosePhase;
  error: string | null;
}

class ConnDiagnose {
  /** "Run the checks again" in an existing thread, when asked. */
  consent = $state<{ conn: Connection; probe: DiagnoseProbe | null } | null>(null);
  active = $state<Active | null>(null);
  /** The side panel is open (Close hides it; the thread stays). */
  shown = $state(false);
  maximized = $state(false);
  /** The embed was seen working since the diagnosis started. */
  private sawWork = false;

  /** An entry point's "Diagnose with an agent…". The same connection's
   *  diagnosis that is already shown just shows again. */
  request(conn: Connection, probe: DiagnoseProbe | null): void {
    if (this.active?.conn.id === conn.id && this.active.phase !== 'failed') {
      this.shown = true;
      return;
    }
    void this.open(conn, probe);
  }

  cancel(): void {
    this.consent = null;
  }

  /** Consent given: run the checks again in the existing thread. */
  async confirm(): Promise<void> {
    const c = this.consent;
    this.consent = null;
    const a = this.active;
    if (c && a?.conn.id === c.conn.id && a.sessionId) await this.sendAgain(a.sessionId, c.conn, c.probe);
  }

  /** Retry after a failure: the lookup again (or the send, with a thread). */
  async retry(): Promise<void> {
    const a = this.active;
    if (!a) return;
    if (a.sessionId) await this.sendAgain(a.sessionId, a.conn, a.probe);
    else await this.open(a.conn, a.probe);
  }

  close(): void {
    this.shown = false;
    this.maximized = false;
  }

  /** The draft composer started the thread (its first message is sent). */
  started(session: RuntimeSession): void {
    const a = this.active;
    if (!a) return;
    this.sawWork = false;
    this.active = { ...a, sessionId: session.id, phase: 'running', error: null };
  }

  /** New thread (/new): close this one (it stays in Threads › History) and
   *  go back to the start composer, where another agent can be picked. */
  async newThread(id: string): Promise<void> {
    const a = this.active;
    if (!a || a.sessionId !== id) return;
    try {
      const port = await loadIdeRuntimeSessionPort();
      if (port.runtimeThreads.get(id)?.status !== 'closed') await port.runtimeThreads.close(id);
      if (this.active?.sessionId === id) this.active = { ...this.active, sessionId: null, phase: 'ready', error: null };
    } catch (e) {
      if (this.active?.sessionId === id) this.active = { ...this.active, phase: 'failed', error: loadErrorText(e) };
    }
  }

  /** The embed's status (`onstatus`): working → idle is a finished diagnosis. */
  status(st: RuntimeEmbedStatus): void {
    const a = this.active;
    if (!a?.sessionId) return;
    if (st === 'working' || st === 'needs-you') {
      this.sawWork = true;
      if (a.phase !== 'running') this.active = { ...a, phase: 'running' };
    } else if (a.phase === 'running' && this.sawWork) {
      this.active = { ...a, phase: 'finished' };
    }
  }

  private async open(conn: Connection, probe: DiagnoseProbe | null): Promise<void> {
    const workspaceId = ws.currentId ?? conn.workspace_id ?? '';
    this.active = { conn, probe, workspaceId, sessionId: null, phase: 'looking', error: null };
    this.shown = true;
    this.sawWork = false;
    try {
      if (!workspaceId) throw new Error('Pick a workspace first — the diagnosis thread belongs to one.');
      const port = await loadIdeRuntimeSessionPort();
      // The lookup is a convenience: when it fails, a new thread works too.
      const found = await port.runtimeThreads.findByOrigin(workspaceId, DIAGNOSE_SOURCE, conn.id).catch(() => null);
      if (this.active?.conn.id !== conn.id) return;
      this.active = { ...this.active, sessionId: found?.id ?? null, phase: 'ready' };
      if (found) this.consent = { conn, probe };
    } catch (e) {
      if (this.active?.conn.id === conn.id) this.active = { ...this.active, phase: 'failed', error: loadErrorText(e) };
    }
  }

  /** The checks again in `id`: reopened when closed, queued behind a running turn. */
  private async sendAgain(id: string, conn: Connection, probe: DiagnoseProbe | null): Promise<void> {
    const a = this.active;
    if (!a) return;
    this.active = { ...a, phase: 'running', error: null };
    this.sawWork = false;
    try {
      const port = await loadIdeRuntimeSessionPort();
      const row = port.runtimeThreads.get(id) ?? (await port.runtimeThreads.loadOne(id));
      if (row.status === 'closed') await port.runtimeThreads.reopen(id);
      await port.runtimeThreads.prompt(id, { text: diagnosePrompt(conn, probe), access_mode: 'approval-required', delivery: 'queue' });
    } catch (e) {
      if (this.active?.sessionId === id) this.active = { ...this.active, phase: 'failed', error: loadErrorText(e) };
    }
  }
}

export const connDiagnose = new ConnDiagnose();
