import { test } from 'node:test';
import assert from 'node:assert/strict';
import { deferred, loadSource } from './sourceHarness.ts';

function fixture(api: Record<string, unknown>) {
  const ws = { currentId: 'ws' };
  const { product } = loadSource(new URL('../src/lib/stores/product.svelte.ts', import.meta.url), {
    '../api/client': { api }, '../loadError': { loadErrorText: String },
    './workspace.svelte': { ws }, '../lazyModule': { announceModule() {} },
  });
  product.selectedId = 'A';
  product.detail = detail('A');
  return { product, ws };
}
const detail = (id: string) => ({ story: { id, title: id }, source: { body_md: id } });

test('late draft mutation cannot replace a newly selected story', async () => {
  const a = deferred<unknown>();
  const { product } = fixture({ patch: () => a.promise, get: async () => detail('B') });
  const saving = product.updateDraft({ body_md: 'edited A' });
  await product.select('B');
  a.resolve(detail('A')); await saving;
  assert.equal(product.detail.story.id, 'B');
  assert.equal(product.detail.source.body_md, 'B');
});

test('late collection response and loading completion belong to their selection', async () => {
  const a = deferred<unknown>(), b = deferred<unknown>();
  const { product } = fixture({ get: (url: string) => url.endsWith('/notes') ? (url.includes('/A/') ? a.promise : b.promise) : Promise.resolve(detail('B')) });
  const first = product.loadNotes();
  await product.select('B');
  const second = product.loadNotes();
  a.resolve([{ id: 'A-note' }]); await first;
  assert.equal(product.notes.length, 0);
  assert.equal(product.loadingNotes, true);
  b.resolve([{ id: 'B-note' }]); await second;
  assert.equal(product.notes[0].id, 'B-note');
});

test('late workspace list response cannot replace the current workspace', async () => {
  const first = deferred<unknown>();
  const { product, ws } = fixture({ get: (url: string) => url.includes('/ws/') ? first.promise : Promise.resolve([{ id: 'new' }]) });
  const pending = product.loadStories(); ws.currentId = 'new-ws';
  await product.loadStories(); first.resolve([{ id: 'old' }]); await pending;
  assert.equal(product.stories[0].id, 'new');
});

test('dirty Product story, tab and view transitions respect Keep editing', async () => {
  const { product } = fixture({ get: async () => detail('B') });
  let allow = false;
  product.registerDraftLeave(async () => allow);
  await product.select('B'); await product.changeTab('notes'); await product.changeView('learnings');
  assert.equal(product.selectedId, 'A'); assert.equal(product.tab, 'overview'); assert.equal(product.view, 'stories');
  allow = true; await product.select('B'); await product.changeTab('notes');
  assert.equal(product.selectedId, 'B'); assert.equal(product.tab, 'notes');
});

test('A to B to A cannot revive a request from the earlier A lifetime', async () => {
  const first = deferred<unknown>();
  const { product } = fixture({ patch: () => first.promise, get: async (url: string) => detail(url.endsWith('/A') ? 'A' : 'B') });
  const pending = product.updateDraft({body_md:'old request'});
  await product.select('B'); await product.select('A');
  first.resolve({story:{id:'A'},source:{body_md:'old request'}}); await pending;
  assert.equal(product.detail.source.body_md, 'A');
});
