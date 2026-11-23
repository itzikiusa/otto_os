// The canvas's pane strip and the multi-thread fan-outs
// (modules/threads/runtime/paneStrip.ts): chip states, the council row and
// which of its actions are enabled (with the reason when not), seat labels
// with and without `meta.council.seat`, "Open council side by side" with
// four seats, broadcast targets and the result toast, the add-a-thread
// slot's order, and the council routes' errors as sentences.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { register } from 'node:module';
import type { ProviderTurn, RuntimeSession, Session, TurnItem } from '../src/lib/api/types';

const hook =
  'export async function resolve(s, c, n) {' +
  "  if (/^\\.{1,2}\\//.test(s) && !/\\.[a-z]+$/i.test(s)) { try { return await n(s + '.ts', c); } catch {} }" +
  '  return n(s, c);' +
  '}';
register('data:text/javascript,' + encodeURIComponent(hook));

const ps = await import('../src/modules/threads/runtime/paneStrip.ts');
const { emptyProjection } = await import('../src/modules/threads/runtime/projection.ts');

const at = (m: number): string => `2026-11-16T10:${String(m).padStart(2, '0')}:00Z`;
function row(id: string, extra: Partial<RuntimeSession> = {}): RuntimeSession {
  return {
    id, workspace_id: 'w', provider: 'claude', title: id, cwd: '/w', status: 'idle', access_mode: 'approval-required',
    created_by: 'u', created_at: at(0), last_active_at: at(0), last_sequence: 0,
    capabilities: { steer: true, images: true, files: true }, ...extra,
  } as RuntimeSession;
}
const seatMeta = (seat: number, extra: Record<string, unknown> = {}, role: 'seat' | 'synthesis' = 'seat') => ({ council: { id: 'c1', seat, role, ...extra }, read_only: true });
const namer: import('../src/modules/threads/runtime/paneStrip.ts').SeatNamer = (p, m, e) => ({ model: m ? m.toUpperCase() : p === 'claude' ? 'Claude' : 'Codex', effort: e ? e[0].toUpperCase() + e.slice(1) : null });

function answered(text: string, tokens = 0) {
  const p = emptyProjection();
  p.turns = [{ id: 't1', ordinal: 1, status: 'completed', started_at: at(1), completed_at: at(2), turn_token_usage: tokens ? { input_tokens: tokens, output_tokens: 0 } : null } as unknown as ProviderTurn];
  p.items = [{ id: 'i1', turn_id: 't1', ordinal: 0, status: 'completed', type: 'assistant_message', text, streaming: false, message_id: 'm', title: null, started_at: at(1), completed_at: at(2), updated_at: at(2) } as unknown as TurnItem];
  return p;
}

const input = (paneIds: string[], all: RuntimeSession[], projections: Record<string, ReturnType<typeof emptyProjection>> = {}, focusedId: string | null = paneIds[0]) => ({
  paneIds, focusedId, all, projections, needsYou: () => false, legacy: () => null, name: namer,
});

test('runtimeState: the projection decides; without one the row\'s last outcome', () => {
  assert.equal(ps.runtimeState(row('a', { status: 'working' }), null), 'working');
  assert.equal(ps.runtimeState(row('a', { status: 'awaiting_permission' }), null), 'needs-you');
  assert.equal(ps.runtimeState(row('a'), null, true), 'needs-you', 'the operator flag wins');
  assert.equal(ps.runtimeState(row('a'), answered('ok')), 'answered');
  assert.equal(ps.runtimeState(row('a', { last_turn_status: 'failed' }), null), 'failed');
  assert.equal(ps.runtimeState(row('a', { last_turn_status: 'completed' }), null), 'answered');
  assert.equal(ps.runtimeState(row('a', { status: 'dormant' }), null), 'stopped');
  assert.equal(ps.runtimeState(row('a'), null), 'idle', 'an older daemon without last_turn_status: Ready, never "Starting"');
  assert.equal(ps.legacyState('needs-you'), 'needs-you');
  assert.equal(ps.legacyState('running'), 'working');
  assert.equal(ps.legacyState('suspended'), 'stopped');
});

test('seat labels: meta.council.seat, else seat order; effort, model and the synthesis', () => {
  const s1 = row('s1', { meta: seatMeta(1, { provider: 'claude', effort: 'high' }) });
  const s2 = row('s2', { provider: 'codex', parent_id: 's1', meta: seatMeta(2, { model: 'sol' }) });
  const s3 = row('s3', { parent_id: 's1', meta: seatMeta(0) });
  const seats = [s1, s2, s3];
  assert.equal(ps.seatLabel(s1, seats, namer), '1 · Claude · High');
  assert.equal(ps.seatLabel(s1, seats, namer, true), '1 · Claude');
  assert.equal(ps.seatLabel(s2, seats, namer), '2 · SOL');
  assert.equal(ps.seatNo(s3, seats), 3, 'no seat number: its place in seat order');
  const judge = row('j', { parent_id: 's1', meta: seatMeta(0, { provider: 'codex' }, 'synthesis') });
  assert.equal(ps.seatLabel(judge, seats, namer), 'Compared · Codex');
  assert.equal(ps.councilTitle('Cache choice · Seat 3 · Claude High'), 'Cache choice');
  assert.equal(ps.councilTitle('Cache plan · claude'), 'Cache plan', 'titles before seat numbers');
});

test('the strip is the council row only when every pane is one council', () => {
  const s1 = row('s1', { title: 'Cache · Seat 1 · Claude', meta: seatMeta(1) });
  const s2 = row('s2', { parent_id: 's1', status: 'working', meta: seatMeta(2) });
  const s3 = row('s3', { parent_id: 's1', meta: seatMeta(3) });
  const other = row('o', { title: 'Fix the build' });
  const all = [s1, s2, s3, other];
  const mixed = ps.stripModel(input(['s1', 'o'], all));
  assert.equal(mixed.council, null);
  assert.deepEqual(mixed.chips.map((c) => c.label), ['s1', 'Fix the build'].map((t, i) => (i === 0 ? 'Cache · Seat 1 · Claude' : t)));
  const m = ps.stripModel(input(['s1', 's2'], all, { s1: answered('Redis', 1200) }));
  assert.equal(m.council?.title, 'Cache');
  assert.deepEqual(m.chips.map((c) => [c.label, c.state, c.shown]), [
    ['1 · Claude', 'answered', true],
    ['2 · Claude', 'working', true],
    ['3 · Claude', 'idle', false],
  ], 'every seat has a chip, shown or not');
  assert.equal(m.chips[0].tokens, 1200);
  assert.equal(m.tokens, 1200);
  assert.equal(m.chips[0].focused, true);
});

test('Synthesize waits for working seats and for a first answer, and says why', () => {
  const chip = (seat: number, state: import('../src/modules/threads/runtime/paneStrip.ts').PaneState) => ({ id: `s${seat}`, label: '', name: '', provider: 'claude', state, seat, tokens: 0, shown: true, focused: false });
  assert.deepEqual(ps.synthGate([chip(1, 'answered'), chip(2, 'working')]), { enabled: false, reason: 'Waiting for model 2' });
  assert.deepEqual(ps.synthGate([chip(1, 'working'), chip(2, 'needs-you'), chip(3, 'working')]), { enabled: false, reason: 'Waiting for models 1, 2 and 3' });
  assert.deepEqual(ps.synthGate([chip(1, 'failed'), chip(2, 'idle')]), { enabled: false, reason: 'No model has answered yet' });
  assert.deepEqual(ps.synthGate([chip(1, 'failed'), chip(2, 'answered'), { ...chip(0, 'working'), seat: 0 }]), { enabled: true, reason: '' }, 'a running judge does not block a new synthesis');
  assert.deepEqual(ps.stoppable([chip(1, 'answered'), chip(2, 'working'), chip(3, 'needs-you')]).map((c) => c.id), ['s2', 's3']);
});

test('Ask every seat says how many and which restart', () => {
  assert.equal(ps.askSeatsText([row('a'), row('b', { status: 'dormant' }), row('c', { status: 'dormant' }), row('d', { status: 'closed' })]), '3 models (2 stopped — they will restart)');
  assert.equal(ps.askSeatsText([row('a'), row('b')]), '2 models');
  assert.equal(ps.askSeatsText([row('a'), row('b', { status: 'dormant' })]), '2 models (1 stopped — it will restart)');
});

test('side by side with four seats keeps the synthesis and the three most recent seats', () => {
  const seats = [1, 2, 3, 4].map((n) => row(`s${n}`, { last_active_at: at([5, 9, 2, 7][n - 1]) }));
  const judge = row('j');
  assert.deepEqual(ps.sideBySideIds(seats, judge), ['s1', 's2', 's4', 'j']);
  assert.deepEqual(ps.sideBySideIds(seats.slice(0, 3), judge), ['s1', 's2', 's3', 'j']);
  assert.deepEqual(ps.sideBySideIds(seats, null), ['s1', 's2', 's3', 's4']);
});

test('broadcast reaches open threads of the sender\'s folder and workspace only', () => {
  const s = (id: string, extra: Partial<Session> = {}) => ({ id, cwd: '/repo', workspace_id: 'w', archived: false, ...extra });
  const from = s('me');
  const peers = ps.projectPeers(from, [s('me'), s('a'), s('b', { cwd: '/repo/' }), s('c', { cwd: '/other' }), s('d', { archived: true }), s('e', { workspace_id: 'x' })]);
  assert.deepEqual(peers.map((p) => p.id), ['a', 'b']);
});

test('the fan-out toast counts who got it and names who did not', () => {
  assert.deepEqual(ps.sendSummary([{ title: 'A', ok: true }, { title: 'B', ok: true }, { title: 'C', ok: true }]), { tone: 'success', title: 'Sent to 3 threads' });
  assert.deepEqual(ps.sendSummary([{ title: 'A', ok: true }, { title: 'B', ok: false }]), { tone: 'warn', title: 'Sent to 1 thread', body: '1 could not receive it: “B”' });
  assert.equal(ps.sendSummary([{ title: 'A', ok: false }]).tone, 'error');
  assert.equal(ps.sendSummary([]).title, 'No open threads to send to');
  assert.equal(ps.sendSummary([{ title: 'A', ok: true }, { title: 'B', ok: true }], 'seat').title, 'Sent to 2 seats');
});

test('the add-a-thread slot: needs you, then working, then recent; never shown or closed', () => {
  const s = (id: string, status: string, m: number, extra: Partial<Session> = {}) => ({ id, status, last_active_at: at(m), archived: false, ...extra }) as Session;
  const rows = [s('old', 'idle', 1), s('new', 'idle', 9), s('busy', 'working', 2), s('ask', 'idle', 0), s('shown', 'working', 9), s('gone', 'idle', 9, { archived: true }), s('new', 'idle', 9)];
  assert.deepEqual(ps.addCandidates(rows, ['shown'], (id) => id === 'ask').map((x) => x.id), ['ask', 'busy', 'new', 'old']);
});

test('council route errors read as sentences, never "conflict:"', () => {
  const err = (message: string) => ({ status: 409, message });
  assert.equal(ps.councilErrorText(err('conflict: no seat has answered yet — nothing to synthesize')), 'No model has answered yet, so there is nothing to compare.');
  assert.equal(ps.councilErrorText(err('conflict: seat 2 is still working — wait for it or stop it, then synthesize')), 'Seat 2 is still working. Wait for it or stop it, then synthesize.');
  assert.equal(ps.councilErrorText(err('conflict: seat 1 is closed — reopen it, then synthesize')), 'The first compared thread is closed. Reopen it, then compare.');
  assert.equal(ps.councilErrorText(err('conflict: seat 2 (codex): busy')), 'Model 2 (codex): Busy.');
  assert.ok(!/conflict/i.test(ps.councilErrorText(err('conflict: the group is full'))));
});
