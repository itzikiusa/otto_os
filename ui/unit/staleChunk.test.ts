// The once-per-minute automatic reload after a chunk fails to load
// (lib/staleChunk.ts).
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { reloadForStaleChunk, STALE_RELOAD_GUARD_MS } from '../src/lib/staleChunk.ts';

const memory = () => {
  const m = new Map<string, string>();
  return { getItem: (k: string) => m.get(k) ?? null, setItem: (k: string, v: string) => void m.set(k, v) };
};

test('reloads once, not again within the guard, again after it', () => {
  const store = memory();
  let reloads = 0;
  const reload = () => void reloads++;
  const t = Date.UTC(2026, 10, 13);
  assert.equal(reloadForStaleChunk(store, t, reload, true), true);
  assert.equal(reloadForStaleChunk(store, t + STALE_RELOAD_GUARD_MS - 1, reload, true), false);
  assert.equal(reloadForStaleChunk(store, t + STALE_RELOAD_GUARD_MS, reload, true), true);
  assert.equal(reloads, 2);
});

test('never reloads offline or without storage', () => {
  let reloads = 0;
  const reload = () => void reloads++;
  assert.equal(reloadForStaleChunk(memory(), Date.now(), reload, false), false);
  assert.equal(reloadForStaleChunk(null, Date.now(), reload, true), false);
  const broken = { getItem: () => null, setItem: () => { throw new Error('quota'); } };
  assert.equal(reloadForStaleChunk(broken, Date.now(), reload, true), false);
  assert.equal(reloads, 0);
});
