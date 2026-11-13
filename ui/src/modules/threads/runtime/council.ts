// Council (docs/features/threads-structured-runtime.md "Council"): one prompt
// to 2–4 agents side by side, then a judge. The pure parts — the seat
// selection, which threads belong to a council, a seat's numbers and why
// Synthesize is not offered yet. Unit-tested in unit/threadsCouncil.test.ts.
import type { CouncilMeta, ProviderTurn, RuntimeSession } from '../../../lib/api/types';

export const COUNCIL_MIN_SEATS = 2;
export const COUNCIL_MAX_SEATS = 4;

/** A seat picked in the new-thread stage. */
export interface CouncilSeat {
  provider: string;
  model: string | null;
  effort: string | null;
}

const sameSeat = (a: CouncilSeat, b: CouncilSeat): boolean =>
  a.provider === b.provider && (a.model ?? '') === (b.model ?? '') && (a.effort ?? '') === (b.effort ?? '');

/** Add `seat` (up to four) or, when it is already picked, take it out. */
export function toggleSeat(seats: readonly CouncilSeat[], seat: CouncilSeat): CouncilSeat[] {
  if (seats.some((s) => sameSeat(s, seat))) return seats.filter((s) => !sameSeat(s, seat));
  return seats.length >= COUNCIL_MAX_SEATS ? [...seats] : [...seats, seat];
}

/** The agent chip with several picked: "Opus, Sol" / "Opus, Sol, +1". */
export function councilChipLabel(labels: readonly string[]): string {
  return labels.length <= 2 ? labels.join(', ') : `${labels[0]}, ${labels[1]}, +${labels.length - 2}`;
}

/** `meta.council`, when well formed. */
export function councilOf(s: RuntimeSession | null | undefined): CouncilMeta | null {
  const c = s?.meta?.council;
  if (!c || typeof c !== 'object' || typeof c.id !== 'string' || typeof c.seat !== 'number') return null;
  return c.role === 'seat' || c.role === 'synthesis' ? c : null;
}

/** The council `id` belongs to: its seats (seat order) and its latest
 *  synthesis. Closed seats stay (their answers still count). */
export function councilMembers(
  all: readonly RuntimeSession[],
  id: string,
): { meta: CouncilMeta; seats: RuntimeSession[]; synthesis: RuntimeSession | null } | null {
  const me = all.find((s) => s.id === id);
  const meta = councilOf(me);
  if (!me || !meta) return null;
  const root = me.parent_id ?? me.id;
  const members = all.filter((s) => (s.id === root || s.parent_id === root) && councilOf(s)?.id === meta.id);
  const seats = members
    .filter((s) => councilOf(s)?.role === 'seat')
    .sort((a, b) => (councilOf(a)?.seat ?? 0) - (councilOf(b)?.seat ?? 0));
  const synth = members
    .filter((s) => councilOf(s)?.role === 'synthesis')
    .sort((a, b) => b.created_at.localeCompare(a.created_at));
  return { meta, seats, synthesis: synth[0] ?? null };
}

/** Every synthesis of the council `id` belongs to, oldest first. */
export function councilSyntheses(all: readonly RuntimeSession[], id: string): RuntimeSession[] {
  const me = all.find((s) => s.id === id);
  const meta = councilOf(me);
  if (!me || !meta) return [];
  const root = me.parent_id ?? me.id;
  return all
    .filter((s) => (s.id === root || s.parent_id === root) && councilOf(s)?.id === meta.id && councilOf(s)?.role === 'synthesis')
    .sort((a, b) => a.created_at.localeCompare(b.created_at));
}

/** `other` sits in the same council as `self` (a seat or its synthesis).
 *  Inside a council the other seats are the council itself, not "linked
 *  threads": the Related chip and an embed's "N linked" chip leave them out. */
export function sameCouncil(self: RuntimeSession | null | undefined, other: RuntimeSession | null | undefined): boolean {
  const a = councilOf(self);
  return !!a && councilOf(other)?.id === a.id;
}

/** A seat's header numbers: time since its first turn started (to the last
 *  one's end, or now while one runs) and the tokens it used. */
export function seatStats(turns: readonly ProviderTurn[], nowMs: number): { elapsedMs: number | null; tokens: number } {
  let start: number | null = null;
  let end: number | null = null;
  let running = false;
  let tokens = 0;
  for (const t of turns) {
    const s = t.started_at ? Date.parse(t.started_at) : NaN;
    if (!Number.isNaN(s) && (start === null || s < start)) start = s;
    if (t.status === 'pending' || t.status === 'running') running = true;
    const e = t.completed_at ? Date.parse(t.completed_at) : NaN;
    if (!Number.isNaN(e) && (end === null || e > end)) end = e;
    const u = t.turn_token_usage;
    if (u) tokens += (u.input_tokens ?? 0) + (u.output_tokens ?? 0);
  }
  if (start === null) return { elapsedMs: null, tokens };
  const stop = running || end === null ? nowMs : end;
  return { elapsedMs: Math.max(0, stop - start), tokens };
}

export function formatElapsed(ms: number | null): string {
  if (ms === null) return '';
  const s = Math.round(ms / 1000);
  if (s < 60) return `${s}s`;
  const m = Math.floor(s / 60);
  return m < 60 ? `${m}m ${s % 60}s` : `${Math.floor(m / 60)}h ${m % 60}m`;
}

export function formatTokens(n: number): string {
  if (n <= 0) return '';
  if (n < 1000) return `${n} tok`;
  return n < 1_000_000 ? `${(n / 1000).toFixed(n < 10_000 ? 1 : 0)}k tok` : `${(n / 1_000_000).toFixed(1)}M tok`;
}

/** Why Synthesize is not offered yet (null = it is). */
export function synthesizeBlocker(seats: readonly { seat: number; working: boolean }[], synthesisWorking: boolean): string | null {
  if (seats.length === 0) return 'The council has no seats left';
  const busy = seats.filter((s) => s.working).map((s) => s.seat);
  if (busy.length === 1) return `Seat ${busy[0]} is still working`;
  if (busy.length > 1) return `Seats ${busy.join(', ')} are still working`;
  return synthesisWorking ? 'The synthesis is still working' : null;
}
