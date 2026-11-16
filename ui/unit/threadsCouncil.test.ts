// Council (modules/threads/runtime/council.ts): seat selection, membership,
// a seat's header numbers and why Synthesize is not offered yet.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import {
  councilChipLabel,
  councilMembers,
  councilOf,
  formatElapsed,
  formatTokens,
  seatStats,
  synthesizeBlocker,
  toggleSeat,
  type CouncilSeat,
} from '../src/modules/threads/runtime/council.ts';
import type { ProviderTurn, RuntimeSession } from '../src/lib/api/types';

const seat = (provider: string, model: string | null = null, effort: string | null = null): CouncilSeat => ({ provider, model, effort });

function row(id: string, extra: Partial<RuntimeSession> = {}): RuntimeSession {
  return {
    id, workspace_id: 'w', provider: 'claude', title: id, cwd: '/w', status: 'idle', access_mode: 'approval-required',
    created_by: 'u', created_at: '2026-11-12T10:00:00Z', last_active_at: '2026-11-12T10:00:00Z', last_sequence: 0,
    capabilities: { steer: true, images: true, files: true }, ...extra,
  };
}
const cmeta = (seatNo: number, role: 'seat' | 'synthesis' = 'seat', id = 'c1') => ({ council: { id, seat: seatNo, role } });

test('toggleSeat adds up to four seats and takes a picked one out', () => {
  let seats: CouncilSeat[] = [];
  seats = toggleSeat(seats, seat('claude', 'opus'));
  seats = toggleSeat(seats, seat('claude', 'opus', 'high'));
  assert.equal(seats.length, 2, 'the same provider and model with another effort is another seat');
  seats = toggleSeat(seats, seat('codex'));
  seats = toggleSeat(seats, seat('gemini'));
  seats = toggleSeat(seats, seat('grok'));
  assert.equal(seats.length, 4, 'never more than four');
  seats = toggleSeat(seats, seat('codex'));
  assert.deepEqual(seats.map((s) => s.provider), ['claude', 'claude', 'gemini']);
});

test('the agent chip names two seats and counts the rest', () => {
  assert.equal(councilChipLabel(['Opus', 'Sol']), 'Opus, Sol');
  assert.equal(councilChipLabel(['Opus', 'Sol', 'Gemini']), 'Opus, Sol, +1');
  assert.equal(councilChipLabel(['A', 'B', 'C', 'D']), 'A, B, +2');
});

test('councilOf reads only a well-formed meta.council', () => {
  assert.equal(councilOf(row('a', { meta: cmeta(1) }))?.seat, 1);
  assert.equal(councilOf(row('a', { meta: { council: { id: 'c', seat: 'x', role: 'seat' } } as never })), null);
  assert.equal(councilOf(row('a', { meta: { council: { id: 'c', seat: 1, role: 'judge' } } as never })), null);
  assert.equal(councilOf(row('a')), null);
});

test('councilMembers finds the seats in order and the latest synthesis, from any member', () => {
  const all = [
    row('s3', { parent_id: 's1', meta: cmeta(3) }),
    row('s1', { meta: cmeta(1) }),
    row('s2', { parent_id: 's1', meta: cmeta(2) }),
    row('j1', { parent_id: 's1', meta: cmeta(0, 'synthesis'), created_at: '2026-11-12T10:05:00Z' }),
    row('j2', { parent_id: 's1', meta: cmeta(0, 'synthesis'), created_at: '2026-11-12T10:09:00Z' }),
    row('peer', { parent_id: 's1' }),
    row('other', { parent_id: 's1', meta: cmeta(1, 'seat', 'c2') }),
  ];
  const fromSeat = councilMembers(all, 's2');
  assert.deepEqual(fromSeat?.seats.map((s) => s.id), ['s1', 's2', 's3']);
  assert.equal(fromSeat?.synthesis?.id, 'j2');
  assert.deepEqual(councilMembers(all, 'j1')?.seats.map((s) => s.id), ['s1', 's2', 's3']);
  assert.equal(councilMembers(all, 'peer'), null, 'a linked thread outside the council');
});

test('seatStats: elapsed from the first start to the last end (or now while running), tokens summed', () => {
  const t = (status: ProviderTurn['status'], start: string, end: string | null, tokens?: [number, number]): ProviderTurn => ({
    id: start, ordinal: 1, status, started_at: start, completed_at: end,
    ...(tokens ? { turn_token_usage: { input_tokens: tokens[0], output_tokens: tokens[1] } } : {}),
  });
  const done = seatStats([t('completed', '2026-11-12T10:00:00Z', '2026-11-12T10:00:42Z', [1000, 200])], 0);
  assert.deepEqual(done, { elapsedMs: 42_000, tokens: 1200 });
  const nowMs = Date.parse('2026-11-12T10:01:00Z');
  assert.equal(seatStats([t('running', '2026-11-12T10:00:00Z', null)], nowMs).elapsedMs, 60_000);
  assert.deepEqual(seatStats([], nowMs), { elapsedMs: null, tokens: 0 });
  assert.equal(formatElapsed(42_000), '42s');
  assert.equal(formatElapsed(190_000), '3m 10s');
  assert.equal(formatElapsed(null), '');
  assert.equal(formatTokens(1200), '1.2k tok');
  assert.equal(formatTokens(0), '');
});

test('Synthesize waits for every seat and for an earlier synthesis', () => {
  assert.equal(synthesizeBlocker([{ seat: 1, working: false }, { seat: 2, working: true }], false), 'Seat 2 is still working');
  assert.equal(synthesizeBlocker([{ seat: 1, working: true }, { seat: 3, working: true }], false), 'Seats 1, 3 are still working');
  assert.equal(synthesizeBlocker([{ seat: 1, working: false }], true), 'The synthesis is still working');
  assert.equal(synthesizeBlocker([{ seat: 1, working: false }, { seat: 2, working: false }], false), null);
  assert.equal(synthesizeBlocker([], false), 'The council has no seats left');
});
