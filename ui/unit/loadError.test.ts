import test from 'node:test';
import assert from 'node:assert/strict';
import { loadSource } from './sourceHarness.ts';

const { loadErrorText } = loadSource(new URL('../src/lib/loadError.ts', import.meta.url), {}, { TypeError, Error, String });

class FakeApiError extends Error {
  status: number;
  constructor(status: number, message: string) {
    super(message);
    this.status = status;
  }
}

test('maps auth / missing statuses to human causes', () => {
  assert.match(loadErrorText(new FakeApiError(401, 'unauthorized')), /session expired/i);
  assert.match(loadErrorText(new FakeApiError(403, 'forbidden')), /access/);
  assert.match(loadErrorText(new FakeApiError(404, 'not found')), /no longer exists/);
});

test('keeps the daemon message for other statuses, falls back to the code', () => {
  assert.equal(loadErrorText(new FakeApiError(500, 'clickhouse is down')), 'clickhouse is down');
  assert.equal(loadErrorText(new FakeApiError(502, '')), 'The daemon answered 502.');
  // A plain object carrying a status is treated the same (duck-typed ApiError).
  assert.equal(loadErrorText({ status: 503 }), 'The daemon answered 503.');
});

test('an unreachable daemon (fetch TypeError) reads as such', () => {
  assert.equal(loadErrorText(new TypeError('Failed to fetch')), 'Otto can’t reach the daemon.');
});

test('plain errors and junk never render empty', () => {
  assert.equal(loadErrorText(new Error('boom')), 'boom');
  assert.equal(loadErrorText(new Error('')), 'Something went wrong.');
  assert.equal(loadErrorText(undefined), 'Something went wrong.');
  assert.equal(loadErrorText(null), 'Something went wrong.');
  assert.equal(loadErrorText('nope'), 'nope');
});
