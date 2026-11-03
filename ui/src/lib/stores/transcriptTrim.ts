// The live-chat head trim of `transcript.svelte.ts` (re-exported there): a
// pure function over turns + their page bounds, so it carries no store state.
import type { Turn } from '../api/types';

/** A chat left open on a working agent keeps appending turns. Past TURN_CAP
 *  the oldest are dropped down to TURN_KEEP ("Load earlier" pages them back),
 *  so memory and per-delta work stay bounded however long the session runs. */
export const TURN_CAP = 600;
export const TURN_KEEP = 500;

/** Byte budget complements the turn count for tool-heavy conversations. */
const TURN_BYTES_CAP = 8 * 1024 * 1024;
const TURN_BYTES_KEEP = 6 * 1024 * 1024;
const turnCharges = new WeakMap<Turn, number>();
function turnCharge(turn: Turn): number {
  let charge = turnCharges.get(turn);
  if (charge === undefined) {
    charge = JSON.stringify(turn).length * 2;
    turnCharges.set(turn, charge);
  }
  return charge;
}

/**
 * Drop the head of a live turn list that outgrew `cap`. The server pages by
 * RECORD index (`before` = exclusive first-record index), which turns don't
 * carry, so the list may only be cut in front of a turn with a known `before`
 * bound (`bounds`: turn id → a cursor B such that every turn ahead of it
 * started before B). Over-estimated bounds are safe — "Load earlier" dedupes
 * by id. Returns null when nothing can be dropped; otherwise the kept turns
 * and the cursor to page earlier from. `bounds` loses the dropped ids.
 */
export function trimTurnHead(
  turns: Turn[],
  bounds: Map<string, string>,
  cap = TURN_CAP,
  keep = TURN_KEEP,
): { turns: Turn[]; cursor: string; dropped: number } | null {
  const bytes = turns.reduce((total, turn) => total + turnCharge(turn), 0);
  const byteOverflow = bytes > TURN_BYTES_CAP;
  if (turns.length <= cap && !byteOverflow) return null;
  let start = Math.max(1, turns.length - keep);
  if (byteOverflow) {
    let retained = bytes;
    for (let i = 0; i < turns.length - 1; i++) {
      retained -= turnCharge(turns[i]);
      if (retained <= TURN_BYTES_KEEP) { start = Math.max(start, i + 1); break; }
    }
  }
  // A byte-heavy window may need fewer turns. Always retain its newest turn.
  const end = byteOverflow ? turns.length - 1 : turns.length - Math.ceil(keep / 2);
  for (let j = start; j <= end; j++) {
    const cursor = bounds.get(turns[j].id);
    if (cursor === undefined) continue;
    for (let i = 0; i < j; i++) bounds.delete(turns[i].id);
    return { turns: turns.slice(j), cursor, dropped: j };
  }
  return null;
}
