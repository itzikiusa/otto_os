// error-handling-6-01: an assist turn the daemon lost with a restart (turns
// live in its memory only) must not stay "running" forever — Send would stay
// disabled and Studio3D would poll for good.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { busyTurn, LOST_TURN_ERROR, mergeTurns, turnOutcome } from '../src/modules/design-hall/assist/model.ts';

function turn(id: string, over: Record<string, unknown> = {}): any {
  return { turn_id: id, status: 'running', error: null, message: null, started_at: '2026-10-23T10:00:00Z', finished_at: null, ...over };
}

test('an authoritative resync fails a running turn the server no longer knows', () => {
  const local = [turn('t1')];
  const m = mergeTurns(local, [], new Set(['t1']));
  assert.equal(m[0].status, 'failed');
  assert.equal(m[0].error, LOST_TURN_ERROR);
  assert.equal(busyTurn(m), null, 'Send is enabled again');
});

test('the optimistic window is safe: a turn sent after the request went out is kept running', () => {
  const m = mergeTurns([turn('new')], [], new Set());
  assert.equal(m[0].status, 'running');
  assert.equal(mergeTurns([turn('t1')], [])[0].status, 'running', 'a non-authoritative merge keeps it');
});

test('finished local turns the server dropped (it keeps ≤ 20) stay as they were', () => {
  const m = mergeTurns([turn('old', { status: 'done', message: 'ok' })], [], new Set(['old']));
  assert.equal(m[0].status, 'done');
  assert.equal(m[0].error, null);
});

test('Studio3D: a turn missing from a successful poll is lost (ends with an error), not still running', () => {
  assert.deepEqual(turnOutcome(undefined), { kind: 'error', title: 'Otto couldn’t finish', body: LOST_TURN_ERROR });
  assert.equal(turnOutcome(turn('t', { status: 'running' })).kind, 'wait');
  assert.equal(turnOutcome(turn('t', { status: 'starting' })).kind, 'wait');
  assert.equal(turnOutcome(turn('t', { status: 'done' })).kind, 'success');
  assert.equal(turnOutcome(turn('t', { status: 'conflict' })).kind, 'warn');
  assert.deepEqual(turnOutcome(turn('t', { status: 'failed', error: 'boom' })), { kind: 'error', title: 'Otto couldn’t finish', body: 'boom' });
});

test('Studio3D.poll routes every answer through turnOutcome', () => {
  const src = readFileSync(join(import.meta.dirname, '..', 'src', 'modules/design-hall/studio3d/Studio3D.svelte'), 'utf8');
  assert.doesNotMatch(src, /if \(!t \|\| t\.status === 'starting'/, 'a vanished turn must not reschedule the poll');
  assert.match(src, /turnOutcome\(turns\.find/);
});
