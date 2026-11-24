// What the inbox rows need beyond a runtime thread's row (threadInbox.ts):
// the kinds of its pending requests (Approval vs Question) and when its
// running turn started ("Working 18s"). Kept from the live runtime events;
// a thread waiting or working before this tab saw its events is read once
// from its event replay (newest turn only, compact). This window only.
import { api } from '../../lib/api/client';
import type { RuntimeEvent, RuntimeReplay } from '../../lib/api/types';
import { onLive } from '../../lib/live';
import { mcpAsks } from './runtime/mcpAsksStore.svelte';

const MAX_READS = 3;

class InboxLive {
  /** session → pending request id → its kind. */
  kinds = $state<Record<string, Record<string, string>>>({});
  /** session → epoch ms its running turn started. */
  since = $state<Record<string, number>>({});
  private read = new Set<string>();
  /** Reads waiting their turn: at most `MAX_READS` in flight, so a reload
   *  with many waiting threads does not fire one request per row at once. */
  private queue: string[] = [];
  private inflight = 0;
  private refs = 0;
  private stop: (() => void) | null = null;

  fold(ev: RuntimeEvent): void {
    const id = ev.session_id;
    if (ev.type === 'runtime_request.updated') {
      const r = ev.runtime_request;
      const { [r.id]: _drop, ...rest } = this.kinds[id] ?? {};
      this.kinds[id] = r.status === 'pending' ? { ...rest, [r.id]: r.kind } : rest;
    } else if (ev.type === 'provider_turn.updated') {
      const t = ev.provider_turn;
      const at = t.started_at ? Date.parse(t.started_at) : NaN;
      if ((t.status === 'running' || t.status === 'pending') && Number.isFinite(at)) this.since[id] = at;
      else if (t.completed_at) delete this.since[id];
    } else if (ev.type === 'turn.terminal') delete this.since[id];
  }

  /** When `id`'s running turn started (epoch ms): from its live events, else
   *  the row's derived `meta.last_turn_started_at` (a thread already working
   *  when the page loaded) — never a read. */
  workingSince(id: string, meta: Record<string, unknown> | null | undefined): number | null {
    const live = this.since[id];
    if (live) return live;
    const at = typeof meta?.last_turn_started_at === 'string' ? Date.parse(meta.last_turn_started_at) : NaN;
    return Number.isFinite(at) ? at : null;
  }

  /** The kinds of `id`'s pending requests (empty when unknown); a waiting
   *  MCP approval card (`mcpAsks`) counts as a permission request. */
  kindsOf(id: string): string[] {
    const kinds = Object.values(this.kinds[id] ?? {});
    return mcpAsks.waiting(id) ? [...kinds, 'permission'] : kinds;
  }

  /** Read a waiting / working thread's newest turn once, when this tab has
   *  not seen its events (a reload). Called from a row, never in a derive. */
  ensure(id: string): void {
    if (this.read.has(id) || this.kinds[id] || this.since[id]) return;
    this.read.add(id);
    this.queue.push(id);
    this.pump();
  }

  private pump(): void {
    while (this.inflight < MAX_READS && this.queue.length) {
      const id = this.queue.shift()!;
      this.inflight++;
      void this.readOne(id).finally(() => {
        this.inflight--;
        this.pump();
      });
    }
  }

  private readOne(id: string): Promise<void> {
    return api
      .get<RuntimeReplay>(`/runtime-sessions/${encodeURIComponent(id)}/events?after_sequence=0&tail_turns=1&compact=1`)
      .then((r) => {
        if (r.kind === 'events') for (const ev of r.events) this.fold(ev);
        else {
          for (const q of r.snapshot.runtime_requests ?? []) if (q.status === 'pending') this.kinds[id] = { ...this.kinds[id], [q.id]: q.kind };
          const running = (r.snapshot.turns ?? []).find((t) => t.status === 'running' || t.status === 'pending');
          if (running?.started_at) this.since[id] = Date.parse(running.started_at);
        }
      })
      .catch(() => {
        /* the row keeps its plain word */
      });
  }

  /** Listen while the sidebar is mounted; returns the unsubscribe. */
  mount(): () => void {
    const releaseMcp = mcpAsks.hold();
    if (this.refs++ === 0) this.stop = onLive(['agent_runtime_event'], (e) => this.fold(e.event as RuntimeEvent));
    return () => {
      releaseMcp();
      if (--this.refs === 0) {
        this.stop?.();
        this.stop = null;
      }
    };
  }
}

export const inboxLive = new InboxLive();
