import { test } from 'node:test';
import assert from 'node:assert/strict';
import { base64ToBytes, bytesToBase64, decodeBase64Fallback } from '../src/lib/b64.ts';

const sample = (): Uint8Array => {
  const b = new Uint8Array(70_000);
  for (let i = 0; i < b.length; i++) b[i] = (i * 131 + 7) & 0xff;
  return b;
};

test('base64ToBytes round-trips binary data (native fromBase64 when present)', () => {
  const bytes = sample();
  assert.deepEqual(base64ToBytes(bytesToBase64(bytes)), bytes);
  assert.deepEqual(base64ToBytes(''), new Uint8Array(0));
});

test('the atob fallback decodes identically', () => {
  const bytes = sample();
  assert.deepEqual(decodeBase64Fallback(bytesToBase64(bytes)), bytes);
});
