// Council rounds (docs/features/threads-structured-runtime.md "Council"):
// after the first answers the person asks the council again — every seat or
// one — and each seat gets the follow-up as its own thread's next turn. The
// daemon stores the rounds (`GET /runtime-sessions/{id}/council/rounds`);
// which message of a seat answers which round is read off the seat's own
// timeline by the round's text, the same way the daemon reads it for the
// judge (crates/otto-agent-protocol/src/runtime/council/rounds.rs
// `round_messages`). Pure; unit-tested in unit/threadsCouncilRounds.test.ts.
import type { CouncilRound, ProviderTurn, QueuedPrompt, TurnItem } from '../../../lib/api/types';

/** Where one seat stands in one round. */
export type SeatRoundState = 'working' | 'queued' | 'done' | 'interrupted' | 'failed' | 'missing' | 'refused' | 'not-asked';

export interface SeatRound {
  state: SeatRoundState;
  /** The seat's answer in the round (its assistant messages, joined). */
  answer: string;
  /** From the round's first turn starting to its last one ending (or now). */
  elapsedMs: number | null;
  /** Why the round never reached the seat (`refused`). */
  reason?: string;
}

/** Characters of an answer "Ask the others" quotes. */
const QUOTE_MAX_CHARS = 24_000;

/** Which of a seat's messages (`sent`, its top-level user messages in
 *  order) answers each of its rounds (`asked`, in order): each round takes
 *  the first message with its text after the previous round's, and a re-ask
 *  of the same text before the next round's message takes its place. */
export function roundMessages(sent: readonly string[], asked: readonly string[]): (number | null)[] {
  const first: (number | null)[] = [];
  let from = 0;
  for (const a of asked) {
    let hit: number | null = null;
    for (let j = from; j < sent.length; j++) {
      if (sent[j].trim() === a.trim()) {
        hit = j;
        break;
      }
    }
    if (hit !== null) from = hit + 1;
    first.push(hit);
  }
  return asked.map((a, k) => {
    const start = first[k];
    if (start === null) return null;
    const end = first.slice(k + 1).find((x): x is number => x !== null) ?? sent.length;
    for (let j = end - 1; j >= start; j--) if (sent[j].trim() === a.trim()) return j;
    return start;
  });
}

const reached = (r: CouncilRound, seat: number): boolean =>
  r.seats.includes(seat) && !(r.refused ?? []).some((x) => x.seat === seat);

/** Seat `seat`'s place in every round (aligned with `rounds`), read off its
 *  timeline: its items (ordinal order), turns and queue. */
export function seatRounds(
  rounds: readonly CouncilRound[],
  seat: number,
  items: readonly TurnItem[],
  turns: readonly ProviderTurn[],
  queued: readonly QueuedPrompt[],
  nowMs: number,
): SeatRound[] {
  const top = items.filter((i) => !i.parent_item_id);
  const users: { ix: number; text: string }[] = [];
  top.forEach((i, ix) => {
    if (i.type === 'user_message') users.push({ ix, text: i.text });
  });
  const mine = rounds.filter((r) => reached(r, seat));
  const picks = roundMessages(users.map((u) => u.text), mine.map((r) => r.text));
  const byTurn = new Map(turns.map((t) => [t.id, t]));
  return rounds.map((r) => {
    if (!r.seats.includes(seat)) return { state: 'not-asked', answer: '', elapsedMs: null };
    const refusal = (r.refused ?? []).find((x) => x.seat === seat);
    if (refusal) return { state: 'refused', answer: '', elapsedMs: null, reason: refusal.reason };
    const u = picks[mine.indexOf(r)];
    // A seat with no message at all (a council seeded without its
    // question): its whole timeline is round 1.
    const whole = users.length === 0 && r.n === 1;
    if ((u === null || u === undefined) && !whole) {
      const waiting = queued.some((q) => q.text.trim() === r.text.trim());
      return { state: waiting ? 'queued' : 'missing', answer: '', elapsedMs: null };
    }
    const span = u === null || u === undefined ? top : top.slice(users[u].ix, users[u + 1]?.ix ?? top.length);
    const answer = span
      .flatMap((i) => (i.type === 'assistant_message' && i.text.trim() ? [i.text] : []))
      .join('\n\n');
    const ts = [...new Set(span.map((i) => i.turn_id).filter((t): t is string => !!t))]
      .map((t) => byTurn.get(t))
      .filter((t): t is ProviderTurn => !!t)
      .sort((a, b) => a.ordinal - b.ordinal);
    const last = ts[ts.length - 1];
    // No turn on record: answered (a timeline without turns), or not started.
    const state: SeatRoundState = !last
      ? answer ? 'done' : whole ? 'missing' : 'working'
      : last.status === 'pending' || last.status === 'running'
        ? 'working'
        : last.status === 'completed'
          ? 'done'
          : last.status === 'failed'
            ? 'failed'
            : 'interrupted';
    const starts = ts.map((t) => (t.started_at ? Date.parse(t.started_at) : NaN)).filter((n) => !Number.isNaN(n));
    const ends = ts.map((t) => (t.completed_at ? Date.parse(t.completed_at) : NaN)).filter((n) => !Number.isNaN(n));
    const start = starts.length ? Math.min(...starts) : null;
    const stop = state === 'working' || !ends.length ? nowMs : Math.max(...ends);
    return { state, answer, elapsedMs: start === null ? null : Math.max(0, stop - start) };
  });
}

/** How a seat's state in the round reads in its header. */
export function roundStateLabel(s: SeatRoundState): string {
  switch (s) {
    case 'working': return 'Working';
    case 'queued': return 'Queued';
    case 'done': return 'Done';
    case 'interrupted': return 'Interrupted';
    case 'failed': return 'Failed';
    case 'missing': return 'Not delivered';
    case 'refused': return 'Not reached';
    case 'not-asked': return 'Sat out';
  }
}

/** "Ask the others": the follow-up the other seats get about seat `seat`'s
 *  answer (the person sees and can edit it before it is sent). */
export function askOthersText(seat: number, answer: string): string {
  const a = answer.trim();
  const quoted = a.length > QUOTE_MAX_CHARS ? `${a.slice(0, QUOTE_MAX_CHARS)}…` : a;
  return `Seat ${seat} answered:\n\n${quoted}\n\nWhere do you disagree, and why?`;
}

/** Why a follow-up cannot be sent (null = it can). */
export function followUpBlocker(rounds: number, maxRounds: number, targets: number): string | null {
  if (rounds >= maxRounds) return `This council has had its ${maxRounds} rounds — start a new council to keep going`;
  if (targets === 0) return 'Pick a seat to ask';
  return null;
}

/** Why the current round cannot be synthesized yet (null = it can) — the
 *  daemon's own refusals, in its order, so a click never ends in one. A
 *  seat counts as answered when it has any answer text in the round, whole
 *  or cut off by a Stop (the judge is told it may be incomplete); a seat a
 *  restart cut mid-answer has none (streaming text is not kept). Seats
 *  without an answer are not a blocker while another one answered: the
 *  judge is told they are missing. */
export function roundSynthBlocker(
  round: number,
  seats: readonly { seat: number; state: SeatRoundState; answer: string }[],
  synthesisWorking: boolean,
  seatOneClosed = false,
): string | null {
  const asked = seats.filter((s) => s.state !== 'not-asked');
  if (asked.length === 0) return 'The council has no seats left';
  const busy = asked.filter((s) => s.state === 'working' || s.state === 'queued').map((s) => s.seat);
  if (busy.length === 1) return `Seat ${busy[0]} is still working`;
  if (busy.length > 1) return `Seats ${busy.join(', ')} are still working`;
  if (synthesisWorking) return 'The synthesis is still working';
  if (seatOneClosed) return 'Seat 1 is closed — reopen it, then synthesize';
  return asked.some((s) => s.answer.trim()) ? null : `No seat has answered round ${round} yet — nothing to synthesize`;
}

/** The judge's prompt (the synthesis thread's first message, built by the
 *  daemon), folded to one line: how many answers it compares. Null when
 *  `text` is not one. */
export function judgePromptSummary(text: string): string | null {
  if (!text.startsWith('You are the judge of a council')) return null;
  const n = (text.match(/^## Seat \d+ — /gm) ?? []).length;
  return `Asked the judge to compare ${n} ${n === 1 ? 'answer' : 'answers'}`;
}

/** The seats of the round with no answer, for "Missing: seat 2 (interrupted)". */
export function missingSeats(seats: readonly { seat: number; state: SeatRoundState; answer: string }[]): string[] {
  return seats
    .filter((s) => s.state !== 'not-asked' && s.state !== 'working' && s.state !== 'queued' && !s.answer.trim())
    .map((s) => `Seat ${s.seat} (${roundStateLabel(s.state).toLowerCase()})`);
}

/** The round a synthesis judged: the one listing it, else round 1 (a
 *  synthesis from before rounds were recorded). */
export function synthesisRound(rounds: readonly CouncilRound[], synthesisId: string): number {
  return rounds.find((r) => (r.syntheses ?? []).includes(synthesisId))?.n ?? 1;
}

/** "All seats" / "Seat 2" / "Seats 1, 3". */
export function targetLabel(targets: readonly number[], seatCount: number): string {
  if (targets.length === 0) return 'No seat';
  if (targets.length === seatCount) return 'All seats';
  return targets.length === 1 ? `Seat ${targets[0]}` : `Seats ${targets.join(', ')}`;
}

/** The first line of a round's text, for its heading. */
export function roundHeading(r: CouncilRound, max = 80): string {
  const line = r.text.split('\n').map((l) => l.trim()).find((l) => l) ?? '';
  return line.length > max ? `${line.slice(0, max)}…` : line;
}
