// DetailOpen (lib/components/side/detailOpen.svelte.ts): a Detail pane opens
// beside the content by default and as a sheet only when asked; asking never
// changes the inline preference, and a remembered preference survives a reload.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { loadSource } from './sourceHarness.ts';

function load(seed: Record<string, string> = {}) {
  const m = new Map(Object.entries(seed));
  const mod = loadSource(new URL('../src/lib/components/side/detailOpen.svelte.ts', import.meta.url), {
    '../../storage': { lsGet: (k: string) => m.get(k) ?? null, lsSet: (k: string, v: string) => void m.set(k, v) },
  });
  return { DetailOpen: mod.DetailOpen as new (key?: string) => {
    open: boolean; sheet: boolean; inline: boolean; asked: boolean;
    present(s: boolean): void; set(o: boolean): void; toggle(): void;
  }, m };
}

test('inline it starts open; as a sheet it is closed until asked', () => {
  const { DetailOpen } = load();
  const d = new DetailOpen();
  assert.equal(d.open, true);
  d.present(true);
  assert.equal(d.open, false, 'a sheet (phone, phone on its side, no room) starts closed');
  d.toggle();
  assert.equal(d.open, true, 'asked');
  assert.equal(d.inline, true);
  d.set(false);
  assert.equal(d.open, false);
});

test('asking for the sheet does not change the inline preference; a new presentation forgets the ask', () => {
  const { DetailOpen } = load();
  const d = new DetailOpen();
  d.set(false); // closed beside the content
  d.present(true);
  d.set(true); // asked for the sheet
  d.present(false);
  assert.equal(d.open, false, 'back inline: the closed preference');
  d.present(true);
  assert.equal(d.open, false, 'the ask is gone');
});

test('a key remembers the inline preference (only that)', () => {
  const { DetailOpen, m } = load({ k: '0' });
  const d = new DetailOpen('k');
  assert.equal(d.open, false);
  d.set(true);
  assert.equal(m.get('k'), '1');
  d.present(true);
  d.set(false);
  assert.equal(m.get('k'), '1', 'closing the sheet is not a preference');
});
