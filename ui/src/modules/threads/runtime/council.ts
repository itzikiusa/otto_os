// Council (docs/features/threads-structured-runtime.md "Council"): one prompt
// to 2–4 agents, each its own thread, then a judge. The pure parts — the seat
// selection and which threads belong to a council. Unit-tested in
// unit/threadsCouncil.test.ts.
import type { CouncilMeta, RuntimeSession } from '../../../lib/api/types';

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
