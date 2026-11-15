// The client-side projection of one structured runtime session: folds the
// sequenced `agent_runtime_event` stream (docs/contracts/ws.md) and the
// `GET /runtime-sessions/{id}/events` replay into entity lists.
//
// Ported from T3 Code (MIT, Copyright (c) 2026 T3 Tools Inc.):
// - packages/client-runtime/src/state/orchestrationV2Projection.ts — entities
//   are upserted WHOLE by id; a streaming item is searched from the end;
// - packages/client-runtime/src/state/threads.ts (`applyEventsLocked`) — an
//   event at or below the applied sequence is skipped.
// What the view derives from it lives in timeline.ts (loaded with the view).
// Otto's sequences are contiguous within a daemon run, so a jump means a
// lost event: the caller replays from `lastSequence` (`gap`).

import type {
  PlanArtifact,
  ProviderSession,
  ProviderThread,
  ProviderTurn,
  QueuedPrompt,
  RuntimeEvent,
  RuntimeReplay,
  RuntimeRequest,
  RuntimeSnapshot,
  TurnItem,
} from '../../../lib/api/types';

export interface RuntimeProjection {
  lastSequence: number;
  session: ProviderSession | null;
  thread: ProviderThread | null;
  turns: ProviderTurn[];
  /** Ordered by `ordinal`. */
  items: TurnItem[];
  requests: RuntimeRequest[];
  plans: PlanArtifact[];
  /** Messages queued behind the running turn (`queue.updated`), oldest first. */
  queued: QueuedPrompt[];
  /** Live `command_output.delta` text per command item (never persisted). */
  output: Record<string, string>;
  /** True when the daemon holds older turns before `oldestTurnOrdinal`. */
  hasOlder: boolean;
  /** Lowest turn `ordinal` loaded into `turns` (or `null` when none). */
  oldestTurnOrdinal: number | null;
  /** Count of older turns still unloaded on the daemon. */
  olderTurns: number;
}

export function emptyProjection(): RuntimeProjection {
  return {
    lastSequence: 0,
    session: null,
    thread: null,
    turns: [],
    items: [],
    requests: [],
    plans: [],
    queued: [],
    output: {},
    hasOlder: false,
    oldestTurnOrdinal: null,
    olderTurns: 0,
  };
}

function minTurnOrdinal(turns: readonly ProviderTurn[]): number | null {
  let min: number | null = null;
  for (let i = 0; i < turns.length; i++) {
    const ord = turns[i].ordinal;
    if (min === null || ord < min) min = ord;
  }
  return min;
}

export function fromSnapshot(snapshot: RuntimeSnapshot, sequence: number): RuntimeProjection {
  const turns = [...snapshot.turns].sort((a, b) => a.ordinal - b.ordinal);
  return {
    lastSequence: sequence,
    session: snapshot.provider_session,
    thread: snapshot.provider_thread,
    turns,
    items: [...snapshot.turn_items].sort((a, b) => a.ordinal - b.ordinal),
    requests: snapshot.runtime_requests,
    plans: snapshot.plans,
    queued: snapshot.queued_prompts ?? [],
    output: {},
    hasOlder: !!snapshot.has_older,
    oldestTurnOrdinal: snapshot.oldest_turn_ordinal ?? minTurnOrdinal(turns),
    olderTurns: snapshot.older_turns ?? 0,
  };
}

function mergeById<T extends { id: string }>(older: readonly T[], newer: readonly T[]): T[] {
  if (older.length === 0) return [...newer];
  const seen = new Set(newer.map((x) => x.id));
  const prefix = older.filter((x) => !seen.has(x.id));
  return prefix.length === 0 ? [...newer] : [...prefix, ...newer];
}

/** Prepend an older history page (`GET …/events?before_turn_ordinal=…`) onto
 *  the live projection without losing newer turns/items or mutating `lastSequence`. */
export function mergeHistoryPage(p: RuntimeProjection, page: RuntimeSnapshot): RuntimeProjection {
  const turns = mergeById(page.turns, p.turns).sort((a, b) => a.ordinal - b.ordinal);
  const items = mergeById(page.turn_items, p.items).sort((a, b) => a.ordinal - b.ordinal);
  const requests = mergeById(page.runtime_requests, p.requests);
  const plans = mergeById(page.plans, p.plans);
  return {
    ...p,
    turns,
    items,
    requests,
    plans,
    hasOlder: !!page.has_older,
    oldestTurnOrdinal: page.oldest_turn_ordinal ?? minTurnOrdinal(turns),
    olderTurns: page.older_turns ?? 0,
  };
}

function upsert<T extends { id: string }>(list: T[], next: T): T[] {
  const i = list.findIndex((x) => x.id === next.id);
  if (i < 0) return [...list, next];
  const out = list.slice();
  out[i] = next;
  return out;
}

function upsertItem(items: TurnItem[], item: TurnItem): TurnItem[] {
  // Streamed updates land on the newest items: search from the end.
  let i = items.length - 1;
  while (i >= 0 && items[i].id !== item.id) i--;
  const out = items.slice();
  if (i >= 0) {
    out[i] = item;
    return out;
  }
  let at = out.length;
  while (at > 0 && out[at - 1].ordinal > item.ordinal) at--;
  out.splice(at, 0, item);
  return out;
}

function applyOne(p: RuntimeProjection, ev: RuntimeEvent): RuntimeProjection {
  const next = { ...p, lastSequence: ev.sequence };
  switch (ev.type) {
    case 'provider_session.updated':
      return { ...next, session: ev.provider_session };
    case 'provider_thread.updated':
      return { ...next, thread: ev.provider_thread };
    case 'provider_turn.updated': {
      const prev = p.turns.find((t) => t.id === ev.provider_turn.id);
      const usage = ev.provider_turn.turn_token_usage ?? prev?.turn_token_usage;
      return { ...next, turns: upsert(p.turns, usage ? { ...ev.provider_turn, turn_token_usage: usage } : ev.provider_turn) };
    }
    case 'turn_item.updated':
      return { ...next, items: upsertItem(p.items, ev.turn_item) };
    case 'runtime_request.updated':
      return { ...next, requests: upsert(p.requests, ev.runtime_request) };
    case 'plan.updated':
      return { ...next, plans: upsert(p.plans, ev.plan) };
    case 'queue.updated':
      return { ...next, queued: ev.queued_prompts };
    case 'command_output.delta':
      return { ...next, output: { ...p.output, [ev.item_id]: (p.output[ev.item_id] ?? '') + ev.chunk } };
    case 'turn.terminal':
      return { ...next, turns: p.turns.map((t) => (t.id === ev.turn_id ? { ...t, status: ev.status } : t)) };
  }
}

/** Fold sequenced events. Events at or below `lastSequence` are duplicates
 *  and skipped; a jump past `lastSequence + 1` stops the fold and reports
 *  `gap` (the caller replays from the returned `lastSequence`). */
export function applyEvents(
  p: RuntimeProjection,
  events: readonly RuntimeEvent[],
): { projection: RuntimeProjection; gap: boolean } {
  let cur = p;
  for (const ev of [...events].sort((a, b) => a.sequence - b.sequence)) {
    if (ev.sequence <= cur.lastSequence) continue;
    if (ev.sequence !== cur.lastSequence + 1) return { projection: cur, gap: true };
    cur = applyOne(cur, ev);
  }
  return { projection: cur, gap: false };
}

/** Apply a `GET …/events?after_sequence=` answer. Replayed events are
 *  authoritative for their range, so they may skip ahead (a daemon restart
 *  resumes sequences above a reserved floor). */
export function applyReplay(p: RuntimeProjection, replay: RuntimeReplay): RuntimeProjection {
  if (replay.kind === 'snapshot') {
    if (replay.snapshot_sequence < p.lastSequence) return p;
    const fresh = fromSnapshot(replay.snapshot, replay.snapshot_sequence);
    if (
      fresh.hasOlder &&
      fresh.oldestTurnOrdinal !== null &&
      p.oldestTurnOrdinal !== null &&
      p.oldestTurnOrdinal < fresh.oldestTurnOrdinal &&
      !p.turns.some((pt) => fresh.turns.some((ft) => ft.ordinal === pt.ordinal && ft.id !== pt.id))
    ) {
      const turnFloor = fresh.oldestTurnOrdinal;
      const itemFloor = fresh.items.length > 0 ? fresh.items[0].ordinal : Infinity;
      const keptTurns = p.turns.filter((t) => t.ordinal < turnFloor);
      const keptTurnIds = new Set(keptTurns.map((t) => t.id));
      const keptItems = p.items.filter(
        (it) => it.ordinal < itemFloor && (!it.turn_id || keptTurnIds.has(it.turn_id)),
      );
      const keptRequestIds = new Set(
        keptItems.flatMap((it) => (it.type === 'approval_request' ? [it.request_id] : [])),
      );
      const keptRequests = p.requests.filter(
        (r) => (r.turn_id != null && keptTurnIds.has(r.turn_id)) || keptRequestIds.has(r.id),
      );
      const keptPlans = p.plans.filter((pl) => pl.turn_id != null && keptTurnIds.has(pl.turn_id));
      return {
        ...fresh,
        turns: mergeById(keptTurns, fresh.turns).sort((a, b) => a.ordinal - b.ordinal),
        items: mergeById(keptItems, fresh.items).sort((a, b) => a.ordinal - b.ordinal),
        requests: mergeById(keptRequests, fresh.requests),
        plans: mergeById(keptPlans, fresh.plans),
        hasOlder: p.hasOlder,
        oldestTurnOrdinal: p.oldestTurnOrdinal,
        olderTurns: p.olderTurns,
      };
    }
    return fresh;
  }
  let cur = p;
  for (const ev of [...replay.events].sort((a, b) => a.sequence - b.sequence)) {
    if (ev.sequence > cur.lastSequence) cur = applyOne(cur, ev);
  }
  return cur.lastSequence < replay.last_sequence ? { ...cur, lastSequence: replay.last_sequence } : cur;
}
