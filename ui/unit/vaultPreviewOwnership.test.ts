import {test} from 'node:test';
import assert from 'node:assert/strict';
import {loadSource, deferred} from './sourceHarness.ts';

function fixture(read: (...args: unknown[]) => Promise<unknown>) {
  const vault = {wsId: 'ws', current: {id: 1}, notePath: 'open.md', status: {generation: '1'}};
  const timers: Array<() => void> = [];
  const {linkPreview} = loadSource(new URL('../src/modules/vault/previewStore.svelte.ts', import.meta.url), {
    '../../lib/api/vault': {vaultNote: read}, './structuredNote': {previewText: (text: string) => text}, './vault.svelte': {vault},
  }, {setTimeout: (run: () => void) => {timers.push(run); return timers.length;}, clearTimeout() {}});
  const anchor = {getBoundingClientRect: () => ({left: 0, top: 0, bottom: 10})};
  return {vault, preview: linkPreview, timers, anchor};
}
const note = (title: string) => ({meta: {title, description: '', okf_type: null}, raw: title});

test('preview at the same path belongs to the newly selected vault', async () => {
  const a = deferred<unknown>(), b = deferred<unknown>(); let reads = 0;
  const {vault, preview, timers, anchor} = fixture(() => ++reads === 1 ? a.promise : b.promise);
  preview.show(anchor, 'same.md'); timers.shift()!();
  vault.current = {id: 2}; preview.show(anchor, 'same.md'); timers.shift()!();
  b.resolve(note('Current vault')); await Promise.resolve(); await Promise.resolve();
  a.resolve(note('Old vault')); await Promise.resolve(); await Promise.resolve();
  assert.equal(preview.current.title, 'Current vault');
});

test('delayed preview never reads the old path in another vault', () => {
  let reads = 0;
  const {vault, preview, timers, anchor} = fixture(async () => {reads++; return note('Wrong');});
  preview.show(anchor, 'old.md'); vault.current = {id: 2}; timers.shift()!();
  assert.equal(reads, 0); assert.equal(preview.current, null);
});

test('preview refreshes after the content generation changes', async () => {
  let reads = 0;
  const {vault, preview, timers, anchor} = fixture(async () => note(`Version ${++reads}`));
  preview.show(anchor, 'same.md'); timers.shift()!(); await Promise.resolve(); await Promise.resolve();
  vault.status.generation = '2';
  preview.show(anchor, 'same.md'); timers.shift()!(); await Promise.resolve(); await Promise.resolve();
  assert.equal(reads, 2); assert.equal(preview.current.title, 'Version 2');
});
