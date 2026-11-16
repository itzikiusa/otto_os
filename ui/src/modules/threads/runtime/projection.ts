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
  ProviderCommand,
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
  /** The agent's own slash commands (`commands.updated`). */
  commands: ProviderCommand[];
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
    commands: [],
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
    commands: snapshot.commands ?? [],
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

export const MAX_STREAM_OUTPUT_BYTES = 128 * 1024;
const STREAM_OUTPUT_TRUNC_MARKER = '…[truncated]\n';

function appendBoundedOutput(prev: string | undefined, chunk: string): string {
  const combined = (prev ?? '') + chunk;
  if (combined.length <= MAX_STREAM_OUTPUT_BYTES) return combined;
  const keep = Math.max(0, MAX_STREAM_OUTPUT_BYTES - STREAM_OUTPUT_TRUNC_MARKER.length);
  return STREAM_OUTPUT_TRUNC_MARKER + combined.slice(combined.length - keep);
}

function upsertMut<T extends { id: string }>(list: T[], next: T): void {
  const i = list.findIndex((x) => x.id === next.id);
  if (i < 0) list.push(next);
  else list[i] = next;
}

export function replaceHydratedItem(p: RuntimeProjection, item: TurnItem): RuntimeProjection {
  let mergedItem = item;
  if (item.type === 'file_change' && !item.diff_str) {
    const prev = p.items.find((x) => x.id === item.id);
    if (prev?.type === 'file_change' && prev.diff_str) {
      mergedItem = { ...item, diff_str: prev.diff_str };
    }
  }
  const items = p.items.slice();
  upsertItemMut(items, mergedItem);
  return { ...p, items };
}

function preserveHydratedItems(prevItems: readonly TurnItem[], nextItems: TurnItem[]): TurnItem[] {
  if (prevItems.length === 0 || nextItems.length === 0) return nextItems;
  let hydrated: Map<string, TurnItem> | null = null;
  for (let i = 0; i < prevItems.length; i++) {
    const it = prevItems[i];
    if (!it.detail_omitted) {
      hydrated ??= new Map();
      hydrated.set(it.id, it);
    }
  }
  if (!hydrated) return nextItems;
  let changed = false;
  const merged = nextItems.map((it) => {
    if (!it.detail_omitted) return it;
    const prev = hydrated!.get(it.id);
    if (!prev || prev.updated_at !== it.updated_at) return it;
    changed = true;
    return prev;
  });
  return changed ? merged : nextItems;
}

function upsertItemMut(items: TurnItem[], item: TurnItem): void {
  // Streamed updates land on the newest items: search from the end.
  let i = items.length - 1;
  while (i >= 0 && items[i].id !== item.id) i--;
  if (i >= 0) {
    items[i] = item;
    return;
  }
  let at = items.length;
  while (at > 0 && items[at - 1].ordinal > item.ordinal) at--;
  items.splice(at, 0, item);
}

function sortedEvents(events: readonly RuntimeEvent[]): readonly RuntimeEvent[] {
  for (let i = 1; i < events.length; i++) {
    if (events[i].sequence < events[i - 1].sequence) {
      return [...events].sort((a, b) => a.sequence - b.sequence);
    }
  }
  return events;
}

function foldEventBatch(
  p: RuntimeProjection,
  events: readonly RuntimeEvent[],
  strictContiguous: boolean,
): { projection: RuntimeProjection; gap: boolean } {
  if (events.length === 0) return { projection: p, gap: false };
  const ordered = sortedEvents(events);

  let lastSequence = p.lastSequence;
  let session = p.session;
  let thread = p.thread;
  let turns = p.turns;
  let turnsCloned = false;
  let items = p.items;
  let itemsCloned = false;
  let requests = p.requests;
  let requestsCloned = false;
  let plans = p.plans;
  let plansCloned = false;
  let queued = p.queued;
  let commands = p.commands;
  let output = p.output;
  let outputCloned = false;
  let mutated = false;
  let gap = false;

  for (let idx = 0; idx < ordered.length; idx++) {
    const ev = ordered[idx];
    if (ev.sequence <= lastSequence) continue;
    if (strictContiguous && ev.sequence !== lastSequence + 1) {
      gap = true;
      break;
    }
    lastSequence = ev.sequence;
    mutated = true;
    switch (ev.type) {
      case 'provider_session.updated':
        session = ev.provider_session;
        break;
      case 'provider_thread.updated':
        thread = ev.provider_thread;
        break;
      case 'provider_turn.updated': {
        if (!turnsCloned) {
          turns = turns.slice();
          turnsCloned = true;
        }
        const prev = turns.find((t) => t.id === ev.provider_turn.id);
        const usage = ev.provider_turn.turn_token_usage ?? prev?.turn_token_usage;
        upsertMut(turns, usage ? { ...ev.provider_turn, turn_token_usage: usage } : ev.provider_turn);
        break;
      }
      case 'turn_item.updated': {
        if (!itemsCloned) {
          items = items.slice();
          itemsCloned = true;
        }
        upsertItemMut(items, ev.turn_item);
        const st = ev.turn_item.status;
        const settled = st !== 'running' && st !== 'pending' && st !== 'waiting';
        if (
          settled &&
          'output' in ev.turn_item &&
          ev.turn_item.output !== undefined &&
          ev.turn_item.output !== null &&
          ev.turn_item.id in output
        ) {
          if (!outputCloned) {
            output = { ...output };
            outputCloned = true;
          }
          delete output[ev.turn_item.id];
        }
        break;
      }
      case 'runtime_request.updated':
        if (!requestsCloned) {
          requests = requests.slice();
          requestsCloned = true;
        }
        upsertMut(requests, ev.runtime_request);
        break;
      case 'plan.updated':
        if (!plansCloned) {
          plans = plans.slice();
          plansCloned = true;
        }
        upsertMut(plans, ev.plan);
        break;
      case 'queue.updated':
        queued = ev.queued_prompts;
        break;
      case 'commands.updated':
        commands = ev.commands;
        break;
      case 'command_output.delta':
        if (!outputCloned) {
          output = { ...output };
          outputCloned = true;
        }
        output[ev.item_id] = appendBoundedOutput(output[ev.item_id], ev.chunk);
        break;
      case 'turn.terminal':
        if (!turnsCloned) {
          turns = turns.slice();
          turnsCloned = true;
        }
        for (let i = 0; i < turns.length; i++) {
          if (turns[i].id === ev.turn_id) turns[i] = { ...turns[i], status: ev.status };
        }
        break;
    }
  }

  if (!mutated) return { projection: p, gap };
  return {
    projection: {
      ...p,
      lastSequence,
      session,
      thread,
      turns,
      items,
      requests,
      plans,
      queued,
      commands,
      output,
    },
    gap,
  };
}

/** Fold sequenced events in a single batched pass. Events at or below
 *  `lastSequence` are duplicates and skipped; a jump past `lastSequence + 1`
 *  stops the fold and reports `gap` (the caller replays from `lastSequence`). */
export function applyEvents(
  p: RuntimeProjection,
  events: readonly RuntimeEvent[],
): { projection: RuntimeProjection; gap: boolean } {
  return foldEventBatch(p, events, true);
}

/** A snapshot carries no live command output (it is never persisted): keep
 *  what was streamed so far for commands the snapshot says still run without
 *  a settled `output`, so a reconnect mid-command does not blank its output. */
function keepLiveOutput(prev: Record<string, string>, items: readonly TurnItem[]): Record<string, string> {
  const ids = Object.keys(prev);
  if (ids.length === 0) return {};
  const kept: Record<string, string> = {};
  const running = new Map<string, TurnItem>();
  for (const it of items) if (it.status === 'running') running.set(it.id, it);
  for (const id of ids) {
    const it = running.get(id);
    const settled = it && 'output' in it && it.output !== undefined && it.output !== null;
    if (it && !settled) kept[id] = prev[id];
  }
  return kept;
}

/** Apply a `GET …/events?after_sequence=` answer. Replayed events are
 *  authoritative for their range, so they may skip ahead (a daemon restart
 *  resumes sequences above a reserved floor). */
export function applyReplay(p: RuntimeProjection, replay: RuntimeReplay): RuntimeProjection {
  if (replay.kind === 'snapshot') {
    if (replay.snapshot_sequence < p.lastSequence) return p;
    const rawFresh = fromSnapshot(replay.snapshot, replay.snapshot_sequence);
    const fresh =
      p.items.length > 0
        ? {
            ...rawFresh,
            items: preserveHydratedItems(p.items, rawFresh.items),
            output: keepLiveOutput(p.output, rawFresh.items),
          }
        : rawFresh;
    if (
      fresh.hasOlder &&
      fresh.oldestTurnOrdinal !== null &&
      p.oldestTurnOrdinal !== null &&
      p.oldestTurnOrdinal < fresh.oldestTurnOrdinal &&
      !p.turns.some((pt) => fresh.turns.some((ft) => ft.ordinal === pt.ordinal && ft.id !== pt.id))
    ) {
      const turnFloor = fresh.oldestTurnOrdinal;
      const keptTurns = p.turns.filter((t) => t.ordinal < turnFloor);
      let maxPrevOrdinal = -Infinity;
      for (let i = 0; i < keptTurns.length; i++) {
        if (keptTurns[i].ordinal > maxPrevOrdinal) maxPrevOrdinal = keptTurns[i].ordinal;
      }
      if (keptTurns.length > 0 && maxPrevOrdinal + 1 >= turnFloor) {
        const itemFloor = fresh.items.length > 0 ? fresh.items[0].ordinal : Infinity;
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
    }
    return fresh;
  }
  const cur = foldEventBatch(p, replay.events, false).projection;
  return cur.lastSequence < replay.last_sequence ? { ...cur, lastSequence: replay.last_sequence } : cur;
}
