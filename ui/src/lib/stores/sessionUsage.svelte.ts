// Per-session tokens + cost for the Agents page (review A5): the pane details
// (one session, `GET /sessions/{id}/usage`, read only when someone looks) and
// the inbox's Tokens sort (one workspace, `GET /workspaces/{wid}/sessions/usage`).
// Both reads are throttled to once a minute per key — each one is a ClickHouse
// query on the daemon (which caches the workspace rollup for 60 s as well);
// `invalidate` (a turn ended) lets the next look read at once.
// A failed read keeps the last value and stays silent: usage is a nicety,
// never a reason for an error toast.
import { api } from '../api/client';
import type { Id, SessionTotals, WorkspaceSessionsUsage } from '../api/types';

const MIN_INTERVAL_MS = 60_000;

class SessionUsageStore {
  /** Per-session totals; `null` = loaded, nothing recorded. */
  bySession: Record<Id, SessionTotals | null> = $state({});
  /** Workspace rollup: session id → totals (sessions with usage only). */
  byWorkspace: Record<Id, Record<Id, SessionTotals>> = $state({});
  /** False once the daemon said usage tracking is unavailable. */
  available = $state(true);

  private lastSession = new Map<Id, number>();
  private lastWorkspace = new Map<Id, number>();

  /** Load one session's totals (throttled; `force` skips the throttle). */
  async loadSession(id: Id, force = false): Promise<void> {
    const t = Date.now();
    if (!force && t - (this.lastSession.get(id) ?? 0) < MIN_INTERVAL_MS) return;
    this.lastSession.set(id, t);
    try {
      const totals = await api.get<SessionTotals | null>(`/sessions/${encodeURIComponent(id)}/usage`);
      this.bySession = { ...this.bySession, [id]: totals ?? null };
    } catch {
      /* forbidden / offline — keep what we had */
    }
  }

  /** The totals moved (a turn ended): the next `loadSession` reads fresh. */
  invalidate(id: Id): void {
    this.lastSession.delete(id);
  }

  /** Load the workspace rollup the inbox sorts by (throttled). */
  async loadWorkspace(wid: Id, force = false): Promise<void> {
    const t = Date.now();
    if (!force && t - (this.lastWorkspace.get(wid) ?? 0) < MIN_INTERVAL_MS) return;
    this.lastWorkspace.set(wid, t);
    try {
      const r = await api.get<WorkspaceSessionsUsage>(`/workspaces/${encodeURIComponent(wid)}/sessions/usage?days=30`);
      this.available = r.available;
      const map: Record<Id, SessionTotals> = {};
      for (const row of r.sessions) map[row.session_id] = row;
      this.byWorkspace = { ...this.byWorkspace, [wid]: map };
    } catch {
      /* keep what we had */
    }
  }

  /** session id → total tokens over the window, for {@link applyTokenOrder}. */
  tokensFor(wid: Id | null): Record<Id, number> {
    const map = wid ? this.byWorkspace[wid] : undefined;
    const out: Record<Id, number> = {};
    if (map) for (const [id, row] of Object.entries(map)) out[id] = row.total_tokens;
    return out;
  }
}

export const sessionUsage = new SessionUsageStore();
