// The canvas's pane strip (ThreadCanvas › PaneStrip.svelte): one chip per
// pane in Split / Grid — its state in one word, click to focus — and, when
// every pane belongs to one council, the council's own row: its seats (shown
// or not), Ask every seat, Synthesize (with the reason when it can't yet),
// Stop all and the result actions. Also the pure halves of "Message every
// pane…", "All open threads in this project" (threadRelay.ts) and the
// add-a-thread slot. Pure — unit-tested in unit/threadsPaneStrip.test.ts.
import type { RuntimeSession, Session } from '../../../lib/api/types';
import { loadErrorText } from '../../../lib/loadError';
import { childState } from './childThreads';
import { councilMembers, councilOf } from './council';
import type { RuntimeProjection } from './projection';
import { threadTokenTotal } from './turnUsage';

/** What a pane is doing, in the strip's words. */
export type PaneState = 'working' | 'needs-you' | 'answered' | 'failed' | 'stopped' | 'idle';

export const PANE_STATE_LABEL: Readonly<Record<PaneState, string>> = {
  working: 'Working',
  'needs-you': 'Needs you',
  answered: 'Done',
  failed: 'Failed',
  stopped: 'Stopped',
  idle: 'Done',
};

const OUTCOME: Readonly<Record<string, PaneState>> = { completed: 'answered', failed: 'failed', interrupted: 'stopped' };

/** A runtime thread's state. With its projection open, the shared
 *  `childState` decides; without one, the row's own last outcome
 *  (`last_turn_status`, absent on an older daemon). */
export function runtimeState(s: RuntimeSession, p: RuntimeProjection | null | undefined, needsYou = false): PaneState {
  if (needsYou) return 'needs-you';
  const tone = childState(s, p).tone;
  if (tone === 'needs-you') return 'needs-you';
  if (tone === 'working' || tone === 'background' || s.status === 'starting') return 'working';
  if (!p) {
    const known = s.last_turn_status ? OUTCOME[s.last_turn_status] : undefined;
    return known ?? (s.status === 'closed' || s.status === 'dormant' ? 'stopped' : 'idle');
  }
  if (tone === 'failed') return 'failed';
  if (tone === 'done') return 'answered';
  if (tone === 'stopped') return 'stopped';
  return 'idle';
}

/** A terminal thread's state from the shared `sessionState` key. */
export function legacyState(key: string): PaneState {
  if (key === 'needs-you') return 'needs-you';
  if (key === 'working' || key === 'running') return 'working';
  if (key === 'failed') return 'failed';
  if (key === 'ended' || key === 'suspended') return 'stopped';
  return 'idle';
}

/** How a seat's model and effort read ("Opus", "High"); the host passes the
 *  provider-aware formatter (lib/providerModels). */
export type SeatNamer = (provider: string, model: string | null, effort: string | null) => { model: string; effort: string | null };

/** The seat's number: `meta.council.seat`, else its place in seat order. */
export function seatNo(s: RuntimeSession, seats: readonly RuntimeSession[]): number {
  const n = councilOf(s)?.seat ?? 0;
  if (n > 0) return n;
  const i = seats.findIndex((x) => x.id === s.id);
  return i >= 0 ? i + 1 : 0;
}

/** "2 · Opus · High" / "Compared · Claude" (`short`: without the effort). */
export function seatLabel(s: RuntimeSession, seats: readonly RuntimeSession[], name: SeatNamer, short = false): string {
  const c = councilOf(s);
  const provider = c?.provider || (typeof s.meta?.provider_slug === 'string' ? s.meta.provider_slug : '') || s.provider;
  const { model, effort } = name(provider, c?.model ?? s.model ?? null, c?.effort ?? null);
  if (c?.role === 'synthesis') return `Compared · ${model}`;
  const head = `${seatNo(s, seats)} · ${model}`;
  return effort && !short ? `${head} · ${effort}` : head;
}

/** The council's own title from seat 1's ("Cache plan · Seat 1 · Claude",
 *  before seat numbers "Cache plan · claude") — the daemon's `council_base`. */
export function councilTitle(seat1Title: string): string {
  const at = seat1Title.lastIndexOf(' · Seat ');
  if (at > 0) return seat1Title.slice(0, at);
  const dot = seat1Title.lastIndexOf(' · ');
  return dot > 0 ? seat1Title.slice(0, dot) : seat1Title;
}

/** The council every one of `paneIds` belongs to (null when any pane is
 *  outside it, or there are none). */
export function paneCouncil(paneIds: readonly string[], all: readonly RuntimeSession[]): ReturnType<typeof councilMembers> {
  if (!paneIds.length) return null;
  const c = councilMembers(all, paneIds[0]);
  if (!c) return null;
  const byId = new Map(all.map((s) => [s.id, s]));
  return paneIds.every((id) => councilOf(byId.get(id))?.id === c.meta.id) ? c : null;
}

export interface StripChip {
  id: string;
  /** "Seat 2 · Opus", or the thread's title. */
  label: string;
  /** The full name for the tooltip ("Seat 2 · Opus · High"). */
  name: string;
  provider: string;
  state: PaneState;
  /** Seat number; 0 = the synthesis; null outside a council. */
  seat: number | null;
  /** Reported tokens of its loaded turns (0 = none known). */
  tokens: number;
  shown: boolean;
  focused: boolean;
}

export interface StripInput {
  paneIds: readonly string[];
  focusedId: string | null;
  all: readonly RuntimeSession[];
  projections: Readonly<Record<string, RuntimeProjection | undefined>>;
  needsYou: (id: string) => boolean;
  /** A terminal pane: its title, provider and `sessionState` key. */
  legacy: (id: string) => { title: string; provider: string; stateKey: string } | null;
  name: SeatNamer;
}

export interface StripModel {
  council: { id: string; title: string; seats: RuntimeSession[]; synthesis: RuntimeSession | null } | null;
  chips: StripChip[];
  /** Tokens of every chip that reported some (0 = none known). */
  tokens: number;
}

const tokensOf = (p: RuntimeProjection | null | undefined): number => (p ? threadTokenTotal(p.turns) : 0);

/** The strip for `paneIds`: a council's seats (+ its latest synthesis) when
 *  all panes are one council, else one chip per pane. */
export function stripModel(inp: StripInput): StripModel {
  const byId = new Map(inp.all.map((s) => [s.id, s]));
  const c = paneCouncil(inp.paneIds, inp.all);
  const chipOf = (s: RuntimeSession, label: string, name: string, seat: number | null): StripChip => ({
    id: s.id,
    label,
    name,
    provider: (typeof s.meta?.provider_slug === 'string' && s.meta.provider_slug) || s.provider,
    state: runtimeState(s, inp.projections[s.id], inp.needsYou(s.id)),
    seat,
    tokens: tokensOf(inp.projections[s.id]),
    shown: inp.paneIds.includes(s.id),
    focused: inp.focusedId === s.id,
  });
  let chips: StripChip[];
  if (c) {
    const members = [...c.seats, ...(c.synthesis ? [c.synthesis] : [])];
    // An older judge shown in a pane keeps its chip too.
    for (const id of inp.paneIds) if (!members.some((m) => m.id === id) && byId.get(id)) members.push(byId.get(id)!);
    chips = members.map((s) => {
      const role = councilOf(s)?.role;
      return chipOf(s, seatLabel(s, c.seats, inp.name, true), seatLabel(s, c.seats, inp.name), role === 'synthesis' ? 0 : seatNo(s, c.seats));
    });
  } else {
    chips = inp.paneIds.flatMap((id): StripChip[] => {
      const s = byId.get(id);
      if (s) return [chipOf(s, s.title, s.title, null)];
      const l = inp.legacy(id);
      if (!l) return [];
      return [{ id, label: l.title, name: l.title, provider: l.provider, state: legacyState(l.stateKey), seat: null, tokens: 0, shown: true, focused: inp.focusedId === id }];
    });
  }
  return {
    council: c ? { id: c.meta.id, title: councilTitle(c.seats[0]?.title ?? ''), seats: c.seats, synthesis: c.synthesis } : null,
    chips,
    tokens: chips.reduce((n, x) => n + x.tokens, 0),
  };
}

const busy = (x: StripChip): boolean => x.state === 'working' || x.state === 'needs-you';
const seatList = (ns: number[]): string => (ns.length === 1 ? `model ${ns[0]}` : `models ${ns.slice(0, -1).join(', ')} and ${ns.at(-1)}`);

/** Whether Synthesize can run now, and why not: every seat must be done
 *  (the daemon refuses while one works) and at least one must have answered. */
export function synthGate(chips: readonly StripChip[]): { enabled: boolean; reason: string } {
  const seats = chips.filter((x) => x.seat !== null && x.seat > 0);
  const waiting = seats.filter(busy).map((x) => x.seat as number);
  if (waiting.length) return { enabled: false, reason: `Waiting for ${seatList(waiting)}` };
  if (!seats.some((x) => x.state === 'answered')) return { enabled: false, reason: 'No model has answered yet' };
  return { enabled: true, reason: '' };
}

/** The chips Stop all stops: everything still working or waiting. */
export const stoppable = (chips: readonly StripChip[]): StripChip[] => chips.filter(busy);

/** "3 seats (2 stopped — they will restart)": what Ask every seat reaches. */
export function askSeatsText(seats: readonly RuntimeSession[]): string {
  const open = seats.filter((s) => s.status !== 'closed');
  const asleep = open.filter((s) => s.status === 'dormant').length;
  const n = `${open.length} ${open.length === 1 ? 'model' : 'models'}`;
  return asleep ? `${n} (${asleep} stopped — ${asleep === 1 ? 'it' : 'they'} will restart)` : n;
}

/** "Open council side by side": every seat and the synthesis when they fit
 *  in four panes; with four seats, the synthesis and the three seats active
 *  most recently (each seat stays one chip away). */
export function sideBySideIds(seats: readonly RuntimeSession[], synthesis: RuntimeSession | null, max = 4): string[] {
  if (!synthesis || seats.length < max) return [...seats, ...(synthesis ? [synthesis] : [])].slice(0, max).map((s) => s.id);
  const recent = new Set([...seats].sort((a, b) => b.last_active_at.localeCompare(a.last_active_at)).slice(0, max - 1).map((s) => s.id));
  return [...seats.filter((s) => recent.has(s.id)).map((s) => s.id), synthesis.id];
}

/** Which threads "All open threads in this project" reaches from `from`:
 *  the person's open threads in the same folder and workspace, never the
 *  sender. Works for runtime rows (as Threads rows) and terminal threads. */
export function projectPeers<T extends Pick<Session, 'id' | 'cwd' | 'workspace_id' | 'archived'>>(
  from: Pick<Session, 'id' | 'cwd' | 'workspace_id'>,
  rows: readonly T[],
): T[] {
  const dir = (c: string | null | undefined): string => (c ?? '').replace(/\/+$/, '');
  return rows.filter((s) => s.id !== from.id && !s.archived && s.workspace_id === from.workspace_id && dir(s.cwd) === dir(from.cwd));
}

/** The toast after a fan-out: "Sent to 3 threads" · "1 could not receive it: X". */
export function sendSummary(results: readonly { title: string; ok: boolean }[], noun = 'thread'): { tone: 'success' | 'warn' | 'error'; title: string; body?: string } {
  const plural = (n: number): string => `${n} ${n === 1 ? noun : `${noun}s`}`;
  const sent = results.filter((r) => r.ok).length;
  const bad = results.filter((r) => !r.ok).map((r) => `“${r.title}”`);
  if (!results.length) return { tone: 'warn', title: `No open ${noun}s to send to`, body: 'Start or reopen one first.' };
  if (!sent) return { tone: 'error', title: 'Couldn’t send the message', body: `None could receive it: ${bad.join(', ')}` };
  return bad.length
    ? { tone: 'warn', title: `Sent to ${plural(sent)}`, body: `${bad.length} could not receive it: ${bad.join(', ')}` }
    : { tone: 'success', title: `Sent to ${plural(sent)}` };
}

/** The add-a-thread slot's list: threads not shown yet and not closed —
 *  needs you first, then working, then the most recent. */
export function addCandidates(rows: readonly Session[], panes: readonly string[], needsYou: (id: string) => boolean): Session[] {
  const rank = (s: Session): number => (needsYou(s.id) ? 0 : s.status === 'working' || s.status === 'running' ? 1 : 2);
  const seen = new Set(panes);
  return rows
    .filter((s) => !s.archived && !seen.has(s.id) && (seen.add(s.id), true))
    .sort((a, b) => rank(a) - rank(b) || b.last_active_at.localeCompare(a.last_active_at));
}

/** A council route's refusal as a sentence: no `conflict:` prefix, no
 *  daemon shorthand ("seat 2 is still working — wait…"). */
export function councilErrorText(e: unknown): string {
  const raw = loadErrorText(e).replace(/^(conflict|invalid|bad request|error)\s*:\s*/i, '').trim();
  const seat = /^seat (\d+) \(([^)]*)\):\s*([\s\S]*)$/i.exec(raw);
  if (seat) return `Model ${seat[1]} (${seat[2]}): ${sentence(seat[3])}`;
  if (/^no seat has answered/i.test(raw)) return 'No model has answered yet, so there is nothing to compare.';
  const working = /^(seat \d+|the last synthesis) is still working/i.exec(raw);
  if (working) return `${cap(working[1])} is still working. Wait for it or stop it, then synthesize.`;
  if (/^seat 1 is closed/i.test(raw)) return 'The first compared thread is closed. Reopen it, then compare.';
  return sentence(raw);
}

const cap = (s: string): string => s.charAt(0).toUpperCase() + s.slice(1);
function sentence(s: string): string {
  const t = cap(s.trim().replace(/\s+—\s+/g, '. ').replace(/`/g, ''));
  return !t || /[.!?]$/.test(t) ? t : `${t}.`;
}
