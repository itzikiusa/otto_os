// Council rounds (modules/threads/runtime/councilRounds.ts): which message of
// a seat answers which round, each seat's state and time in a round, the
// "Ask the others" draft and why a follow-up / a synthesis is not offered.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import {
  askOthersText,
  followUpBlocker,
  missingSeats,
  roundMessages,
  roundSynthBlocker,
  seatRounds,
  judgePromptSummary,
  synthesisRound,
  targetLabel,
} from '../src/modules/threads/runtime/councilRounds.ts';
import type { CouncilRound, ProviderTurn, QueuedPrompt, TurnItem } from '../src/lib/api/types';

const round = (n: number, text: string, seats: number[], extra: Partial<CouncilRound> = {}): CouncilRound => ({
  n, text, seats, created_at: '2026-11-13T10:00:00Z', ...extra,
});
let ord = 0;
const user = (text: string, turn: string): TurnItem =>
  ({ id: `u${ord}`, turn_id: turn, ordinal: ord++, status: 'completed', title: null, started_at: null, completed_at: null, updated_at: '', type: 'user_message', text }) as unknown as TurnItem;
const said = (text: string, turn: string): TurnItem =>
  ({ id: `a${ord}`, turn_id: turn, ordinal: ord++, status: 'completed', title: null, started_at: null, completed_at: null, updated_at: '', type: 'assistant_message', message_id: `m${ord}`, text, streaming: false }) as unknown as TurnItem;
const turn = (id: string, ordinal: number, status: ProviderTurn['status'], start: string, end: string | null): ProviderTurn => ({
  id, ordinal, status, started_at: `2026-11-13T10:${start}Z`, completed_at: end ? `2026-11-13T10:${end}Z` : null,
});
const NOW = Date.parse('2026-11-13T10:10:00Z');

test('roundMessages: each round takes its own message; a re-ask replaces it; a queued one is not there yet', () => {
  assert.deepEqual(roundMessages(['q1', 'aside', 'q2'], ['q1', 'q2']), [0, 2]);
  assert.deepEqual(roundMessages(['q1', 'q1', 'q2'], ['q1', 'q2']), [1, 2], 'the re-ask of round 1 counts');
  assert.deepEqual(roundMessages(['q1'], ['q1', 'q2']), [0, null]);
  assert.deepEqual(roundMessages(['q', 'again', 'again'], ['q', 'again', 'again']), [0, 1, 2]);
});

test('seatRounds: answers grouped by round, states, sat-out and refused seats, elapsed per round', () => {
  ord = 0;
  const items = [user('Which cache?', 't1'), said('Redis.', 't1'), user('Why?', 't2'), said('Speed', 't2')];
  const turns = [turn('t1', 1, 'completed', '00:00', '00:42'), turn('t2', 2, 'running', '05:00', null)];
  const rounds = [
    round(1, 'Which cache?', [1, 2]),
    round(2, 'Why?', [1, 2]),
    round(3, 'Only seat 2', [2]),
    round(4, 'Again', [1], { refused: [{ seat: 1, reason: 'the seat is closed' }] }),
  ];
  const r = seatRounds(rounds, 1, items, turns, [], NOW);
  assert.equal(r[0].state, 'done');
  assert.equal(r[0].answer, 'Redis.');
  assert.equal(r[0].elapsedMs, 42_000);
  assert.equal(r[1].state, 'working');
  assert.equal(r[1].answer, 'Speed');
  assert.equal(r[1].elapsedMs, 5 * 60_000, 'a running round counts to now');
  assert.equal(r[2].state, 'not-asked', 'seat 1 sat round 3 out');
  assert.equal(r[3].state, 'refused');
  assert.equal(r[3].reason, 'the seat is closed');
});

test('seatRounds: a queued follow-up, a removed one, and a turn cut off by a restart', () => {
  ord = 0;
  const items = [user('q1', 't1'), said('half an ans', 't1')];
  const turns = [turn('t1', 1, 'interrupted', '00:00', '00:10')];
  const queued: QueuedPrompt[] = [{ id: 'qp', text: 'q2', attachments: 0, created_at: '' }];
  const rounds = [round(1, 'q1', [1]), round(2, 'q2', [1]), round(3, 'q3', [1])];
  const r = seatRounds(rounds, 1, items, turns, queued, NOW);
  assert.equal(r[0].state, 'interrupted');
  assert.equal(r[0].answer, 'half an ans', 'a cut-off answer is kept');
  assert.equal(r[1].state, 'queued');
  assert.equal(r[2].state, 'missing');
  // A seat seeded without its question: the whole timeline is round 1.
  ord = 0;
  const seeded = seatRounds([round(1, '', [1])], 1, [said('Use Redis.', 't9')], [], [], NOW);
  assert.equal(seeded[0].state, 'done');
  assert.equal(seeded[0].answer, 'Use Redis.');
  assert.equal(seatRounds([round(1, '', [2])], 2, [], [], [], NOW)[0].state, 'missing');
});

test('askOthersText quotes the answer and asks where they disagree', () => {
  const t = askOthersText(2, '  Use Redis.  ');
  assert.equal(t, 'Seat 2 answered:\n\nUse Redis.\n\nWhere do you disagree, and why?');
  assert.ok(askOthersText(1, 'x'.repeat(30_000)).includes('x'.repeat(24_000) + '…'));
});

test('follow-up and synthesis blockers; missing seats are named, never a blocker', () => {
  assert.match(followUpBlocker(10, 10, 2) ?? '', /had its 10 rounds/);
  assert.equal(followUpBlocker(3, 10, 0), 'Pick a seat to ask');
  assert.equal(followUpBlocker(3, 10, 1), null);

  const seats = [
    { seat: 1, state: 'done' as const, answer: 'A' },
    { seat: 2, state: 'interrupted' as const, answer: '' },
    { seat: 3, state: 'not-asked' as const, answer: '' },
  ];
  assert.equal(roundSynthBlocker(2, seats, false), null);
  assert.deepEqual(missingSeats(seats), ['Seat 2 (interrupted)']);
  assert.equal(roundSynthBlocker(2, [{ seat: 1, state: 'working', answer: '' }, ...seats.slice(1)], false), 'Seat 1 is still working');
  assert.equal(roundSynthBlocker(2, [{ seat: 1, state: 'queued', answer: '' }, { seat: 2, state: 'working', answer: '' }], false), 'Seats 1, 2 are still working');
  assert.equal(roundSynthBlocker(2, seats, true), 'The synthesis is still working');
  assert.equal(
    roundSynthBlocker(4, [{ seat: 1, state: 'interrupted', answer: '' }, { seat: 2, state: 'failed', answer: '' }], false),
    'No seat has answered round 4 yet — nothing to synthesize',
    'a round a restart cut for every seat is refused, as the daemon does',
  );
  assert.equal(roundSynthBlocker(4, [{ seat: 1, state: 'interrupted', answer: 'Redis, beca' }], false), null, 'a cut-off answer counts');
  assert.equal(roundSynthBlocker(2, seats, false, true), 'Seat 1 is closed — reopen it, then synthesize');
});

test('synthesisRound and targetLabel', () => {
  const rounds = [round(1, 'a', [1, 2]), round(2, 'b', [1, 2], { syntheses: ['j2'] })];
  assert.equal(synthesisRound(rounds, 'j2'), 2);
  assert.equal(synthesisRound(rounds, 'old'), 1, 'a synthesis from before rounds judged round 1');
  assert.equal(targetLabel([1, 2, 3], 3), 'All seats');
  assert.equal(targetLabel([2], 3), 'Seat 2');
  assert.equal(targetLabel([1, 3], 3), 'Seats 1, 3');
});

test('judgePromptSummary folds the daemon-built judge prompt to one line', () => {
  const p = 'You are the judge of a council: the same question went to 2 agents\n\n## The question\n\nq\n\n## Seat 1 — claude\n\nA\n\n## Seat 2 — codex (gpt)\n\nB\n';
  assert.equal(judgePromptSummary(p), 'Asked the judge to compare 2 answers');
  assert.equal(judgePromptSummary('You are the judge of a council\n## Seat 3 — gemini\n'), 'Asked the judge to compare 1 answer');
  assert.equal(judgePromptSummary('Which cache?'), null, 'a person’s message is never folded');
});
