// The connection library as every page reads it: ONE cached list of saved
// connection profiles + the shared section tree, fetched once per workspace
// and re-read only when the daemon says it changed (`connections_changed`,
// `resource_access_changed`, or a socket resync that may have lost one).
// Before this, the Connections page, the DB workbench and its dialogs each
// fetched `/workspaces/{id}/connections` (and the sections twice) on mount.
//
// It also remembers the last known health of each connection (a test, an
// open, a failed query) so the list can say "Connected · 14 ms" or "Can't
// connect" from what is already known — the list never tests a connection
// just because it rendered.

import { api } from '../api/client';
import type { Connection, ConnectionSection, Id } from '../api/types';
import { ws } from './workspace.svelte';
import { loadErrorText } from '../loadError';
import { appLive } from '../live';
import { lsGet, lsSet } from '../storage';

/** What the UI last learned about a connection's health. */
export interface ConnCheck {
  ok: boolean;
  /** Why it failed (ok=false), verbatim from the daemon. */
  message?: string;
  latencyMs?: number | null;
  serverVersion?: string | null;
  /** Epoch ms of the observation. */
  at: number;
}

const CHECKS_KEY = 'otto.connections.checks';

function readChecks(): Record<Id, ConnCheck> {
  try {
    const raw = lsGet(CHECKS_KEY);
    const v = raw ? (JSON.parse(raw) as unknown) : null;
    return v && typeof v === 'object' ? (v as Record<Id, ConnCheck>) : {};
  } catch {
    return {};
  }
}

class ConnectionListStore {
  rows: Connection[] = $state([]);
  sections: ConnectionSection[] = $state([]);
  /** The first load for the current workspace has finished (ok or failed). */
  loaded = $state(false);
  loading = $state(false);
  error: string | null = $state(null);
  checks: Record<Id, ConnCheck> = $state(readChecks());

  private wsKey: Id | null = null;
  private listeners = new Set<(rows: Connection[]) => void>();
  private stale = true;
  private inflight: Promise<void> | null = null;
  /** A re-read asked for while one is on the wire (whose answer may predate the change). */
  private queued: Promise<void> | null = null;

  /** Load unless the cached list is current for this workspace. */
  ensure(): Promise<void> {
    const wid = ws.currentId;
    if (!wid) return Promise.resolve();
    if (wid === this.wsKey && !this.stale && this.loaded) return Promise.resolve();
    return this.refresh();
  }

  /** Re-read now; during a read for the same workspace, once more after it. */
  refresh(): Promise<void> {
    const wid = ws.currentId;
    if (!wid) return Promise.resolve();
    if (this.inflight && this.wsKey === wid) return (this.queued ??= this.inflight.then(() => ((this.queued = null), this.refresh())));
    if (this.wsKey !== wid) {
      // Another workspace: never show the previous one's rows as this one's.
      this.wsKey = wid;
      this.loaded = false;
      this.rows = [];
      this.sections = [];
    }
    this.stale = false;
    const p = this.fetch(wid).finally(() => {
      if (this.inflight === p) this.inflight = null;
    });
    this.inflight = p;
    return p;
  }

  private async fetch(wid: Id): Promise<void> {
    this.loading = true;
    try {
      const [rows, sections] = await Promise.all([
        api.get<Connection[]>(`/workspaces/${wid}/connections`),
        // Sections are optional: a failure leaves a flat list, never an error.
        api.get<ConnectionSection[]>(`/workspaces/${wid}/connection-sections`).catch(() => null),
      ]);
      if (wid !== this.wsKey || this.queued) return;
      this.rows = rows;
      if (sections) this.sections = sections;
      this.error = null;
      this.notify();
    } catch (e) {
      if (wid === this.wsKey && !this.queued) this.error = loadErrorText(e);
    } finally {
      if (wid === this.wsKey && !this.queued) {
        this.loading = false;
        this.loaded = true;
      }
    }
  }

  /** The daemon said the library changed: refetch if anyone has read it. */
  invalidate(): void {
    this.stale = true;
    if (this.loaded) void this.refresh();
  }

  /** Called with the rows after every successful (re)load — the workbench
   *  prunes tabs of deleted connections without fetching on its own. */
  onRefreshed(fn: (rows: Connection[]) => void): () => void {
    this.listeners.add(fn);
    return () => this.listeners.delete(fn);
  }

  /** Replace one row in place (after a save/move) without a refetch. */
  upsert(c: Connection): void {
    const i = this.rows.findIndex((x) => x.id === c.id);
    this.rows = i === -1 ? [...this.rows, c] : this.rows.map((x) => (x.id === c.id ? c : x));
    this.notify();
  }

  remove(id: Id): void {
    this.rows = this.rows.filter((x) => x.id !== id);
    this.notify();
  }

  private notify(): void {
    for (const fn of this.listeners) fn(this.rows);
  }

  /** Record what the UI just learned about a connection's health. */
  recordCheck(id: Id, check: Omit<ConnCheck, 'at'> & { at?: number }): void {
    this.checks = { ...this.checks, [id]: { ...check, at: check.at ?? Date.now() } };
    lsSet(CHECKS_KEY, JSON.stringify(this.checks));
  }

  /** A workbench status change (ready / failed) as a check. */
  recordStatus(id: Id, s: { phase: string; error?: string; latencyMs?: number; serverVersion?: string }): void {
    if (s.phase === 'ready') this.recordCheck(id, { ok: true, latencyMs: s.latencyMs ?? null, serverVersion: s.serverVersion ?? null });
    else if (s.phase === 'error') this.recordCheck(id, { ok: false, message: s.error });
  }

  /** The connection was opened just now (keeps "opened 2 min ago" honest
   *  without a refetch; the daemon stamps the same on its side). */
  stampOpened(id: Id): void {
    const now = new Date().toISOString();
    this.rows = this.rows.map((x) => (x.id === id ? { ...x, last_opened_at: now } : x));
  }
}

export const connectionList = new ConnectionListStore();

if (typeof window !== 'undefined') {
  appLive.on(['connections_changed', 'resource_access_changed'], () => connectionList.invalidate());
  appLive.onResync(() => connectionList.invalidate());
}
