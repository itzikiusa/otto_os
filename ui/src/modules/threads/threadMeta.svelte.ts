// The thread meta a person's client owns, on the thread itself so every
// device agrees (docs/contracts/api.md: `thread_pinned`, `thread_history`,
// `thread_seen_at`): a structured thread through `PATCH /runtime-sessions/{id}`
// `{meta}`, a terminal one through `PATCH /sessions/{id}` `{meta}`. Other
// clients hear it live (`provider_session.updated` `thread` /
// `session_meta_updated`). A write shows at once; a failed one rolls back
// with the shared error toast. This browser's old local lists move onto the
// threads once (`migrate`).
import { api } from '../../lib/api/client';
import type { RuntimeEvent, RuntimeSession, Session, ThreadMetaPatch } from '../../lib/api/types';
import { auth } from '../../lib/stores/auth.svelte';
import { ws } from '../../lib/stores/workspace.svelte';
import { onLive } from '../../lib/live';
import { toastError } from '../../lib/toastError';
import { runtimeThreads } from './runtime/runtimeThreads.svelte';
import { migrationPatches, unseen } from './threadInbox';

type Key = keyof ThreadMetaPatch;
const THREAD_KEYS: readonly Key[] = ['thread_pinned', 'thread_history', 'thread_seen_at'];

const MIGRATED = 'otto_threads_meta_migrated';

class ThreadMeta {
  /** Writes in flight, shown before the daemon answers. */
  overlay = $state<Record<string, ThreadMetaPatch>>({});

  private session(id: string): Session | null {
    return ws.getSession(id) ?? ws.otherWsSessions.find((s) => s.id === id) ?? ws.archivedSessions.find((s) => s.id === id) ?? null;
  }

  private value(id: string, key: Key): unknown {
    const o = this.overlay[id];
    if (o && key in o) return o[key];
    const rt = runtimeThreads.get(id);
    const meta = (rt ? rt.meta : this.session(id)?.meta) as Record<string, unknown> | null | undefined;
    return meta?.[key];
  }

  pinned(id: string): boolean {
    return this.value(id, 'thread_pinned') === true;
  }

  history(id: string): boolean {
    return this.value(id, 'thread_history') === true;
  }

  /** When it last came back to the person (the inbox's recency): its last
   *  turn's end, else its last activity. */
  cameBackAt(s: Pick<Session, 'id' | 'last_active_at'>): string {
    return this.endedAt(s.id) ?? s.last_active_at;
  }

  /** When a structured thread's turn ended, seen live here (`turn.terminal`). */
  private endedLive = $state<Record<string, string>>({});

  /** When its last turn ended: a structured thread's `meta.last_turn_ended_at`
   *  (or a later end seen live); a terminal thread's last activity. */
  endedAt(id: string): string | null {
    const rt = runtimeThreads.get(id);
    if (!rt) return this.session(id)?.last_active_at ?? null;
    const row = (rt.meta as Record<string, unknown> | null | undefined)?.last_turn_ended_at;
    const live = this.endedLive[id];
    const fromRow = typeof row === 'string' ? row : null;
    return live && (!fromRow || Date.parse(live) > Date.parse(fromRow)) ? live : fromRow;
  }

  /** Live: another client's pin / History / seen (`provider_session.thread`)
   *  and when a turn ends. Installed while Threads is mounted. */
  mount(focused: () => string | null): () => void {
    // Back to a window that shows a thread: it is seen now.
    const onVisible = (): void => {
      if (!document.hidden) this.markSeen(focused());
    };
    document.addEventListener('visibilitychange', onVisible);
    const off = onLive(['agent_runtime_event'], (e) => {
      const ev = e.event as RuntimeEvent;
      if (ev.type === 'turn.terminal' && (ev.status === 'completed' || ev.status === 'failed')) {
        this.endedLive = { ...this.endedLive, [ev.session_id]: ev.ts };
        // Ended in front of the person: seen (after the row took the outcome).
        if (focused() === ev.session_id) setTimeout(() => this.markSeen(ev.session_id), 0);
      } else if (ev.type === 'provider_session.updated') {
        const row = runtimeThreads.byId[ev.session_id];
        if (!row) return;
        const meta: Record<string, unknown> = { ...(row.meta ?? {}) };
        for (const k of THREAD_KEYS) delete meta[k];
        runtimeThreads.byId[ev.session_id] = { ...row, meta: { ...meta, ...(ev.provider_session.thread ?? {}) } };
      }
    });
    return () => {
      off();
      document.removeEventListener('visibilitychange', onVisible);
    };
  }

  /** Finished since the person last had it in front of them, on any device
   *  (`thread_seen_at`); a thread never marked seen falls back to this
   *  window's own flag (`live`: it saw the turn end off screen). */
  unread(id: string, live: boolean): boolean {
    const seen = this.value(id, 'thread_seen_at');
    return typeof seen === 'string' ? unseen(this.endedAt(id), seen) : live;
  }

  /** The person has the thread in front of them (focused, window visible):
   *  write `thread_seen_at` — once, only when something is new since the
   *  last mark, and only on their own thread. */
  markSeen(id: string | null): void {
    if (!id || (typeof document !== 'undefined' && document.hidden)) return;
    const rt = runtimeThreads.get(id);
    const owner = rt?.created_by ?? this.session(id)?.created_by;
    if (!owner || owner !== auth.me?.id) return;
    const ended = this.endedAt(id);
    const seenRaw = this.value(id, 'thread_seen_at');
    const seen = typeof seenRaw === 'string' ? Date.parse(seenRaw) : NaN;
    if (Number.isFinite(seen) && (!ended || Date.parse(ended) <= seen)) return;
    // Never before what was on screen, whatever this device's clock says.
    // One attempt per (thread, ended turn): a refused or failed write (daemon
    // down, a viewer) must not loop through the effect that called this.
    if (this.tried.get(id) === ended) return;
    this.tried.set(id, ended);
    const at = new Date(Math.max(Date.now(), ended ? Date.parse(ended) + 1 : 0)).toISOString();
    void this.write(id, { thread_seen_at: at }, false);
  }

  /** The last turn end a seen mark was tried for, per thread (not reactive). */
  private tried = new Map<string, string | null>();

  /** Write `patch` on thread `id`; false (rolled back, toast shown) on failure. */
  async write(id: string, patch: ThreadMetaPatch, loud = true): Promise<boolean> {
    this.overlay = { ...this.overlay, [id]: { ...this.overlay[id], ...patch } };
    try {
      if (runtimeThreads.get(id)) {
        const r = await api.patch<RuntimeSession>(`/runtime-sessions/${encodeURIComponent(id)}`, { meta: patch });
        const row = runtimeThreads.byId[id];
        if (row) runtimeThreads.byId[id] = { ...row, meta: r.meta ?? null };
      } else {
        await ws.updateSessionMeta(id, { ...patch });
      }
      return true;
    } catch (e) {
      if (loud) toastError('Couldn’t save that change', e);
      return false;
    } finally {
      const cur = { ...this.overlay[id] };
      for (const k of Object.keys(patch) as Key[]) if (cur[k] === patch[k]) delete cur[k];
      const next = { ...this.overlay };
      if (Object.keys(cur).length) next[id] = cur;
      else delete next[id];
      this.overlay = next;
    }
  }

  /** Once per browser: this browser's old local pins / History onto the
   *  threads that still exist (all of the person's structured threads are
   *  read first). Returns true when done, so the caller stops keeping them. */
  async migrate(pins: readonly string[], history: readonly string[]): Promise<boolean> {
    if (typeof localStorage === 'undefined' || localStorage.getItem(MIGRATED)) return true;
    if (pins.length === 0 && history.length === 0) {
      localStorage.setItem(MIGRATED, '1');
      return true;
    }
    try {
      await runtimeThreads.loadAll();
    } catch {
      return false; // try again next time
    }
    const exists = (id: string): boolean => !!runtimeThreads.get(id) || !!this.session(id);
    await Promise.all(migrationPatches(pins, history, exists).map(([id, patch]) => this.write(id, patch, false)));
    localStorage.setItem(MIGRATED, '1');
    return true;
  }
}

export const threadMeta = new ThreadMeta();
