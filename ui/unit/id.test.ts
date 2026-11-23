import { test } from 'node:test';
import assert from 'node:assert/strict';
import { newId } from '../src/lib/id.ts';

const UUID4 = /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/;

test('newId uses crypto.randomUUID where the page has it', () => {
  assert.match(newId(), UUID4);
});

test('newId still makes a UUID on a plain-http page, where randomUUID does not exist', () => {
  const real = globalThis.crypto;
  Object.defineProperty(globalThis, 'crypto', {
    value: { getRandomValues: <T extends Uint8Array<ArrayBuffer>>(a: T) => real.getRandomValues(a) },
    configurable: true,
  });
  try {
    const ids = new Set(Array.from({ length: 50 }, newId));
    assert.equal(ids.size, 50);
    for (const id of ids) assert.match(id, UUID4);
  } finally {
    Object.defineProperty(globalThis, 'crypto', { value: real, configurable: true });
  }
});
