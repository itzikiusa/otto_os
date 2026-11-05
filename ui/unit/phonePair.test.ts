import { test } from 'node:test';
import assert from 'node:assert/strict';
import {
  continueUrl,
  loadPairState,
  pairUrl,
  parsePairRoute,
  resetPairMemory,
  savePairState,
  secondsLeft,
  validNext,
  validPairCode,
  type PairStorage,
} from '../src/lib/phonePair.ts';

const CODE = 'a'.repeat(43); // 32 random bytes, base64url

function fakeStorage(): PairStorage & { data: Map<string, string> } {
  const data = new Map<string, string>();
  return {
    data,
    getItem: (k) => data.get(k) ?? null,
    setItem: (k, v) => void data.set(k, v),
    removeItem: (k) => void data.delete(k),
  };
}

test('next allow-list: only the in-app session/thread routes survive', () => {
  for (const ok of ['', 'home', 'agents', 'threads', 'agents/01HXYZABC', 'threads/abc-123_X']) {
    assert.equal(validNext(ok), ok, ok);
  }
  assert.equal(validNext('#/threads/T1'), 'threads/T1', 'a leading #/ is tolerated');
  for (const bad of [
    'settings/phone-access',
    's/S1/token',
    'agents/../settings',
    'agents/a/b',
    'threads/',
    'https://evil.example/',
    '//evil.example',
    'agents/' + 'x'.repeat(65),
    'pair/' + CODE,
    'agents/id with space',
  ]) {
    assert.equal(validNext(bad), null, bad);
  }
  assert.equal(validNext(null), null);
  assert.equal(validNext(undefined), null);
});

test('pair route parsing keeps the code and a validated next', () => {
  assert.deepEqual(parsePairRoute(['pair', CODE]), { code: CODE, next: null });
  assert.deepEqual(parsePairRoute(['pair', CODE, 'threads', 'T1']), { code: CODE, next: 'threads/T1' });
  assert.deepEqual(parsePairRoute(['pair', CODE, 'settings', 'users']), { code: CODE, next: null }, 'bad next dropped');
  assert.equal(parsePairRoute(['pair', 'short']), null, 'a short code is not a pairing code');
  assert.equal(parsePairRoute(['pair']), null);
  assert.equal(parsePairRoute(['s', CODE]), null);
  assert.ok(validPairCode('f'.repeat(64)), 'hex codes are accepted too');
  assert.ok(!validPairCode('a/b'.repeat(20)));
});

test('pairing + continue URLs are built from the served URL', () => {
  const base = 'http://mac.tail0b7b6b.ts.net/';
  assert.equal(pairUrl(base, CODE), `http://mac.tail0b7b6b.ts.net/#/pair/${CODE}`);
  assert.equal(pairUrl(base, CODE, 'agents/S1'), `http://mac.tail0b7b6b.ts.net/#/pair/${CODE}/agents/S1`);
  assert.equal(pairUrl(base, CODE, 'settings'), `http://mac.tail0b7b6b.ts.net/#/pair/${CODE}`);
  assert.equal(continueUrl(base, 'threads/T1'), 'http://mac.tail0b7b6b.ts.net/#/threads/T1');
  assert.equal(continueUrl(base, 'users'), null);
});

test('pairing state round-trips through tab storage and re-validates on load', () => {
  const st = fakeStorage();
  resetPairMemory();
  savePairState({ code: CODE, next: 'agents/S1' }, st);
  resetPairMemory();
  assert.deepEqual(loadPairState(st), {
    code: CODE,
    next: 'agents/S1',
    pairingId: undefined,
    pollSecret: undefined,
    confirmCode: undefined,
    expiresAt: undefined,
  });
  // A tampered stored value is re-validated, never trusted.
  st.setItem('otto_pair', JSON.stringify({ code: 'x', next: 'settings/users' }));
  resetPairMemory();
  const s = loadPairState(st);
  assert.equal(s?.code, null);
  assert.equal(s?.next, null);
  savePairState(null, st);
  resetPairMemory();
  assert.equal(loadPairState(st), null);
  assert.equal(st.data.size, 0);
});

test('secondsLeft counts down and floors at zero', () => {
  const t0 = Date.parse('2026-11-05T10:00:00Z');
  assert.equal(secondsLeft('2026-11-05T10:02:00Z', t0), 120);
  assert.equal(secondsLeft('2026-11-05T09:59:00Z', t0), 0);
  assert.equal(secondsLeft(null, t0), 0);
  assert.equal(secondsLeft('garbage', t0), 0);
});
