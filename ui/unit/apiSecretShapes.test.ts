import { test } from 'node:test';
import assert from 'node:assert/strict';
import {
  MASK,
  sensitiveKey,
  stripSecretsForStorage,
  unmaskHistory,
} from '../src/lib/api/apiSecretShapes.ts';

const marker = { $secret: 'otto.api.request.r1' };

test('persisted tabs never carry plaintext credentials', () => {
  const draft = {
    name: 'unsaved',
    auth: { type: 'bearer' as const, token: 'live-token' },
    headers: [
      { key: 'Authorization', value: 'Bearer abc', enabled: true },
      { key: 'X-Trace', value: 'keep-me', enabled: true },
      { key: 'X-Api-Key', value: '{{API_KEY}}', enabled: true },
    ],
    query: [{ key: 'access_token', value: 'q-secret' }],
  };
  const out = stripSecretsForStorage(draft);
  assert.deepEqual(out.auth, { type: 'bearer', token: '' });
  assert.equal(out.headers[0].value, '');
  assert.equal(out.headers[1].value, 'keep-me');
  assert.equal(out.headers[2].value, '{{API_KEY}}', 'variable references carry no secret');
  assert.equal(out.query[0].value, '');
  assert.equal(out.name, 'unsaved', 'non-secret fields untouched');
  // The in-memory draft is not mutated.
  assert.equal(draft.auth.token, 'live-token');
  assert.equal(draft.headers[0].value, 'Bearer abc');
});

test('a saved-request tab persists the stored marker, not the typed replacement', () => {
  const saved = {
    auth: { type: 'bearer' as const, token: marker },
    headers: [{ key: 'Authorization', value: 'Bearer {{TOKEN}}' }],
    query: [],
  };
  const draft = {
    auth: { type: 'bearer' as const, token: 'typed-but-unsaved' },
    headers: [{ key: 'authorization', value: 'Bearer typed' }],
    query: [],
  };
  const out = stripSecretsForStorage(draft, saved);
  assert.deepEqual(out.auth, { type: 'bearer', token: marker });
  assert.equal(out.headers[0].value, 'Bearer {{TOKEN}}');
  // Markers already in the draft are kept as-is.
  const kept = stripSecretsForStorage({ auth: { type: 'bearer' as const, token: marker }, headers: [], query: [] });
  assert.deepEqual(kept.auth, { type: 'bearer', token: marker });
});

test('history replay never sends the literal mask', () => {
  const snap = {
    auth: { type: 'basic' as const, username: 'u', password: MASK },
    headers: [{ key: 'Authorization', value: MASK, enabled: true }, { key: 'Accept', value: 'a/b' }],
    query: [],
  };
  // No saved request: blanked (rows disabled) and reported.
  const blank = unmaskHistory(snap);
  assert.deepEqual(blank.auth, { type: 'basic', username: 'u', password: '' });
  assert.deepEqual(blank.headers[0], { key: 'Authorization', value: '', enabled: false });
  assert.equal(blank.headers[1].value, 'a/b');
  assert.equal(blank.blanked, true);

  // Saved request still exists: its marker / row value comes back.
  const saved = {
    auth: { type: 'basic' as const, username: 'u', password: marker },
    headers: [{ key: 'authorization', value: 'Basic {{CREDS}}' }],
    query: [],
  };
  const full = unmaskHistory(snap, saved);
  assert.deepEqual(full.auth, { type: 'basic', username: 'u', password: marker });
  assert.equal(full.headers[0].value, 'Basic {{CREDS}}');
  assert.equal(full.headers[0].enabled, true);
  assert.equal(full.blanked, false);

  // A different auth type on the saved request can't refill the member.
  const other = unmaskHistory(snap, { auth: { type: 'bearer', token: marker }, headers: [], query: [] });
  assert.equal((other.auth as unknown as { password: string }).password, '');
  assert.equal(other.blanked, true);
});

test('credential-carrying keys mirror the daemon', () => {
  for (const k of ['Authorization', 'Proxy-Authorization', 'Cookie', 'X-API-Key', 'x-auth-token', 'my_token', 'client-secret', 'password']) {
    assert.equal(sensitiveKey(k), true, k);
  }
  for (const k of ['Accept', 'Content-Type', 'X-Trace']) assert.equal(sensitiveKey(k), false, k);
});
