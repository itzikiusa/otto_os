// SSH / custom-CLI terminal tabs in the Connections hub: which tab's session
// has ended, (re)connecting a tab, and closing one. The tab list itself lives
// in the database store (`database.sshTabs`); this owns what the page needs
// on top of it so DatabasePage stays a thin caller.

import { SvelteMap } from 'svelte/reactivity';
import { api } from '../../lib/api/client';
import { confirmer } from '../../lib/confirm.svelte';
import { database } from '../../lib/stores/database.svelte';
import type { Connection, Session, SessionStatus } from '../../lib/api/types';
import { livenessOf, sshCloseQuestion, sshOpenAction, withSession, type SshLiveness } from './ssh-tab-rules';

class SshTabs {
  /** Session id → last liveness the tab's Terminal reported. Only the shown
   *  tab mounts a Terminal, so a background tab keeps its last-known state. */
  private readonly live = new SvelteMap<string, SshLiveness>();

  liveness(sessionId: string): SshLiveness | undefined {
    return this.live.get(sessionId);
  }

  /** The Terminal's `onstatus` (a status frame, or its exit). */
  report(sessionId: string, status: SessionStatus): void {
    this.live.set(sessionId, livenessOf(status));
  }

  /** What opening `connId` from the tree should do. */
  action(connId: string): 'open' | 'focus' | 'reconnect' {
    return sshOpenAction(database.sshTabs, (s) => this.liveness(s), connId);
  }

  /** Spawn a session for `c`: a new tab, or — on an existing tab (Reconnect)
   *  — swapped into that tab in place, retiring the ended session. */
  async open(c: Connection, workspaceId: string): Promise<void> {
    const session = await api.long.post<Session>(`/connections/${c.id}/open`, { workspace_id: workspaceId });
    const tab = database.sshTabs.find((t) => t.connId === c.id);
    if (!tab) {
      // Deliberately NOT registered via `ws.addSession` — that would navigate
      // to the Agents view. The session is killed when the tab closes.
      database.addSshTab({ connId: c.id, sessionId: session.id, name: c.name, kind: c.kind });
      return;
    }
    const old = tab.sessionId;
    database.sshTabs = withSession(database.sshTabs, c.id, session.id);
    database.focusSsh(c.id);
    this.live.delete(old);
    // The ended session is not in the Agents list either: don't let it linger.
    void api.del(`/sessions/${old}`).catch(() => {});
  }

  /** Close a tab and kill its session — asking first while it still runs. */
  async close(connId: string): Promise<void> {
    const tab = database.sshTabs.find((t) => t.connId === connId);
    if (!tab) return;
    const question = sshCloseQuestion(tab, this.liveness(tab.sessionId));
    if (question && !(await confirmer.ask(question, { title: 'Close session', confirmLabel: 'Close', danger: true }))) return;
    database.closeSshTab(connId);
    this.live.delete(tab.sessionId);
    try {
      await api.del(`/sessions/${tab.sessionId}`);
    } catch {
      // best-effort: the tab is gone from the workbench regardless
    }
  }
}

export const sshTabs = new SshTabs();
