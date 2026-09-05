// History store — every past Claude/Codex conversation Otto knows about:
// its own session rows (all statuses, archived included) merged with transcripts
// found on disk that no session claims (`status:'on_disk'`). Backed by
// `GET /workspaces/{wid}/history` (docs/design/conversation-view.md §4.3/§4.6).
//
// Filters are server-side where the API has a knob (q / provider / cwd /
// status) and client-side for the date window. Paging is keyset: `before` is
// the oldest loaded row's `last_active_at`.

import { api } from '../../../lib/api/client';
import { activity } from '../../../lib/stores/activity.svelte';
import type {
  HistoryEntry,
  HistoryImportReq,
  HistoryStatus,
  Session,
} from '../../../lib/api/types';

export type DateWindow = 'all' | 'today' | '7d' | '30d';
export type ProviderFilter = 'all' | 'claude' | 'codex';
export type StatusFilter = 'all' | HistoryStatus;

const PAGE = 100;

/** A repo/cwd bucket in the list — like the Codex/Claude app sidebars. */
export interface HistoryGroup {
  key: string;
  label: string;
  cwd: string;
  entries: HistoryEntry[];
}

/** Stable identity for an entry: the session id, else the transcript path. */
export function entryKey(e: HistoryEntry): string {
  return e.session_id ?? `path:${e.transcript_path}`;
}

/** Row title: the provider's AI title, else the first prompt, else a stub. */
export function entryTitle(e: HistoryEntry): string {
  const t = (e.title ?? '').trim();
  if (t) return t;
  const p = (e.first_prompt ?? '').trim().replace(/\s+/g, ' ');
  if (p) return p.length > 120 ? p.slice(0, 117) + '…' : p;
  return `${e.provider === 'codex' ? 'Codex' : 'Claude'} session`;
}

/** `~`-shortened last two segments of a cwd for group headers. */
export function shortCwd(cwd: string): string {
  const parts = cwd.replace(/\/+$/, '').split('/').filter(Boolean);
  return parts.length <= 2 ? cwd : `…/${parts.slice(-2).join('/')}`;
}

function sinceFor(win: DateWindow): number {
  const now = Date.now();
  switch (win) {
    case 'today': {
      const d = new Date();
      d.setHours(0, 0, 0, 0);
      return d.getTime();
    }
    case '7d':
      return now - 7 * 86_400_000;
    case '30d':
      return now - 30 * 86_400_000;
    default:
      return 0;
  }
}

class HistoryStore {
  entries: HistoryEntry[] = $state([]);
  loading = $state(false);
  loadingMore = $state(false);
  error = $state<string | null>(null);
  hasMore = $state(false);

  q = $state('');
  provider = $state<ProviderFilter>('all');
  status = $state<StatusFilter>('all');
  /** Exact cwd filter ('' = every folder). */
  cwd = $state('');
  date = $state<DateWindow>('all');

  selectedKey = $state<string | null>(null);

  /** Workspace the current list belongs to (reload when it changes). */
  private wsId: string | null = null;
  private seq = 0;

  selected = $derived<HistoryEntry | null>(
    this.entries.find((e) => entryKey(e) === this.selectedKey) ?? null,
  );

  /** Distinct folders in the loaded rows (for the repo filter), most recent first. */
  folders = $derived.by<{ cwd: string; label: string }[]>(() => {
    const seen = new Map<string, string>();
    for (const e of this.entries) {
      if (!seen.has(e.cwd)) seen.set(e.cwd, e.repo_name ?? shortCwd(e.cwd));
    }
    return [...seen].map(([cwd, label]) => ({ cwd, label }));
  });

  /** Client-side date window applied on top of the server filters. */
  visible = $derived.by<HistoryEntry[]>(() => {
    const since = sinceFor(this.date);
    if (!since) return this.entries;
    return this.entries.filter((e) => new Date(e.last_active_at).getTime() >= since);
  });

  /** Grouped by repo/cwd, groups ordered by their most recent entry. */
  groups = $derived.by<HistoryGroup[]>(() => {
    const byCwd = new Map<string, HistoryGroup>();
    for (const e of this.visible) {
      let g = byCwd.get(e.cwd);
      if (!g) {
        g = { key: e.cwd, label: e.repo_name ?? shortCwd(e.cwd), cwd: e.cwd, entries: [] };
        byCwd.set(e.cwd, g);
      }
      g.entries.push(e);
    }
    // Entries arrive newest-first from the API; keep that inside each group and
    // order groups by their first (= newest) row.
    return [...byCwd.values()].sort(
      (a, b) =>
        new Date(b.entries[0].last_active_at).getTime() -
        new Date(a.entries[0].last_active_at).getTime(),
    );
  });

  private query(before?: string): string {
    const p = new URLSearchParams();
    const q = this.q.trim();
    if (q) p.set('q', q);
    if (this.provider !== 'all') p.set('provider', this.provider);
    if (this.status !== 'all') p.set('status', this.status);
    if (this.cwd) p.set('cwd', this.cwd);
    if (before) p.set('before', before);
    p.set('limit', String(PAGE));
    return p.toString();
  }

  /** (Re)load the first page for the current filters. */
  async load(wsId: string): Promise<void> {
    this.wsId = wsId;
    const my = ++this.seq;
    this.loading = true;
    this.error = null;
    try {
      const rows = await api.get<HistoryEntry[]>(`/workspaces/${wsId}/history?${this.query()}`);
      if (my !== this.seq) return; // a newer load superseded this one
      this.entries = rows;
      this.hasMore = rows.length >= PAGE;
      if (this.selectedKey && !rows.some((e) => entryKey(e) === this.selectedKey)) {
        // Keep the selection only while it is in the list (filters may hide it).
        this.selectedKey = null;
      }
    } catch (e) {
      if (my !== this.seq) return;
      this.error = e instanceof Error ? e.message : String(e);
    } finally {
      if (my === this.seq) this.loading = false;
    }
  }

  /** Append the next page (older rows). */
  async loadMore(): Promise<void> {
    if (!this.wsId || this.loadingMore || !this.hasMore || this.entries.length === 0) return;
    const before = this.entries[this.entries.length - 1].last_active_at;
    this.loadingMore = true;
    try {
      const rows = await api.get<HistoryEntry[]>(
        `/workspaces/${this.wsId}/history?${this.query(before)}`,
      );
      const have = new Set(this.entries.map(entryKey));
      this.entries = [...this.entries, ...rows.filter((e) => !have.has(entryKey(e)))];
      this.hasMore = rows.length >= PAGE;
    } catch (e) {
      this.error = e instanceof Error ? e.message : String(e);
    } finally {
      this.loadingMore = false;
    }
  }

  /** Re-run with the current filters (used after filter changes / rescans). */
  refresh(): Promise<void> {
    return this.wsId ? this.load(this.wsId) : Promise.resolve();
  }

  select(e: HistoryEntry | null): void {
    this.selectedKey = e ? entryKey(e) : null;
  }

  /** Kick the background index walk; progress arrives as `history_index_progress`. */
  async rescan(wsId: string): Promise<void> {
    activity.beginHistoryRescan();
    await api.post<void>(`/workspaces/${wsId}/history/rescan`);
  }

  /**
   * Import an `on_disk` transcript as a `reconnectable` Otto session so the
   * existing resume path (`POST /sessions/{id}/restart` → `resume_args`) can
   * continue it. Returns the new session; the row is patched in place.
   */
  async importEntry(wsId: string, e: HistoryEntry): Promise<Session> {
    const body: HistoryImportReq = { provider: e.provider, transcript_path: e.transcript_path };
    const s = await api.post<Session>(`/workspaces/${wsId}/history/import`, body);
    const key = entryKey(e);
    this.entries = this.entries.map((x) =>
      entryKey(x) === key
        ? { ...x, session_id: s.id, status: 'reconnectable', resumable: true }
        : x,
    );
    if (this.selectedKey === key) this.selectedKey = s.id;
    return s;
  }

  /** Reflect a status change (archive/restart) without a refetch. */
  patchSession(sessionId: string, patch: Partial<HistoryEntry>): void {
    this.entries = this.entries.map((x) => (x.session_id === sessionId ? { ...x, ...patch } : x));
  }
}

export const history = new HistoryStore();
