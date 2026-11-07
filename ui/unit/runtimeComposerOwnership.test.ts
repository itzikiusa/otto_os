import { test } from 'node:test';
import assert from 'node:assert/strict';
import { componentFunctions } from './componentFunctions.ts';

const component = new URL('../src/modules/threads/runtime/RuntimeThread.svelte', import.meta.url);

test('reopening A then switching to B sends the waiting prompt only to A', async () => {
  let finish!: () => void;
  const calls: string[] = [];
  const draft = { text: 'for A', images: [], model: null, effort: null, pickedMode: null, sending: false, reopening: false, awaitSeq: null, startError: null };
  const state = componentFunctions(component, ['submit', 'send', 'reopen'], {
    sessionId: 'A', ownerId: 'A', text: 'for A', images: [], model: null, effort: null, pickedMode: null,
    draft, closed: true, reopening: false, sending: false, running: false, awaitSeq: null, startError: null,
    parseInteractiveSlashInput: () => null,
    runtimeThreads: {
      reopen: () => new Promise<void>((resolve) => { finish = resolve; }),
      prompt: async (id: string) => { calls.push(id); return { sequence: 1 }; },
    },
    runtimeDrafts: { owns: () => true },
    toastError: () => {}, ApiError: class extends Error {},
  });
  state.submit();
  state.sessionId = 'B';
  finish();
  await new Promise((resolve) => setTimeout(resolve, 0));
  assert.deepEqual(calls, ['A']);
});

test('runtime drafts retain attachments and choices only for their owning thread', async () => {
  const { importRunes, svelteClient } = await import('./runesHarness.ts');
  const { RuntimeDrafts } = await importRunes<{ RuntimeDrafts: new () => { for(id: string): any } }>(new URL('../src/lib/stores/runtimeDrafts.svelte.ts', import.meta.url));
  const store = new RuntimeDrafts();
  const a = store.for('A');
  const b = store.for('B');
  a.text = 'draft for A';
  a.pickedMode = 'full-access';
  a.model = 'model-a';
  a.images.push({ name: 'a.png', mime: 'image/png', data_base64: 'AA==' });
  assert.equal(b.text, '');
  assert.equal(b.pickedMode, null);
  assert.equal(b.model, null);
  assert.equal(b.images.length, 0);
  assert.equal(store.for('A'), a);
  assert.equal(store.for('A').text, 'draft for A');
  // A file read started before switching still updates the remounted owner's draft.
  const runtime = await svelteClient();
  let count = 0;
  const stop = runtime.effectRoot(() => runtime.effect(() => { count = store.for('A').images.length; }));
  runtime.flushSync();
  a.images.push({ name: 'second.png', mime: 'image/png', data_base64: 'BB==' });
  runtime.flushSync();
  assert.equal(count, 2);
  assert.equal(b.images.length, 0);
  stop();
});

test('a failed A send restores A draft while B stays untouched', async () => {
  let reject!: (e: Error) => void;
  const a = { text: '', images: [{ name: 'a.png', mime: 'image/png', data_base64: 'AA==' }], model: null, effort: null, pickedMode: null, sending: false, awaitSeq: null, startError: null };
  const b = { text: 'draft B', images: [] };
  const calls: string[] = [];
  const state = componentFunctions(component, ['send'], {
    sessionId: 'A', ownerId: 'A', draft: a,
    runtimeThreads: { prompt: (id: string) => { calls.push(id); return new Promise((_resolve, fail) => { reject = fail; }); } },
    runtimeDrafts: { owns: () => true },
    ApiError: class extends Error {}, toastError: () => {},
  });
  const pending = state.send('draft A');
  state.sessionId = 'B';
  reject(new Error('offline'));
  await pending;
  assert.deepEqual(calls, ['A']);
  assert.equal(a.text, 'draft A');
  assert.equal(a.images[0]?.name, 'a.png');
  assert.equal(a.sending, false);
  assert.equal(b.text, 'draft B');
  assert.equal(b.images.length, 0);
});

test('deleted threads and changed identities forget drafts without dropping a same-identity draft', async () => {
  const { importRunes } = await import('./runesHarness.ts');
  const { RuntimeDrafts } = await importRunes<{ RuntimeDrafts: new () => { for(id: string): any; forget(id: string): void; setIdentity(id: string): void } }>(new URL('../src/lib/stores/runtimeDrafts.svelte.ts', import.meta.url));
  const drafts = new RuntimeDrafts();
  drafts.setIdentity('server-a/user-a');
  drafts.for('A').text = 'private';
  drafts.for('A').images.push({ name: 'secret.png', mime: 'image/png', data_base64: 'AA==' });
  drafts.setIdentity('server-a/user-a');
  assert.equal(drafts.for('A').text, 'private');
  drafts.forget('A');
  assert.equal(drafts.for('A').text, '');
  assert.equal(drafts.for('A').images.length, 0);
  drafts.for('A').text = 'another private draft';
  drafts.setIdentity('server-a/user-b');
  assert.equal(drafts.for('A').text, '');
  drafts.for('A').text = 'user B draft';
  drafts.setIdentity('server-b/user-b');
  assert.equal(drafts.for('A').text, '');
});


test('an identity change while reopening cancels the old pending prompt', async () => {
  let finish!: () => void;
  let current = true;
  const calls: string[] = [];
  const draft = { text: 'private A', images: [], model: null, effort: null, pickedMode: null, reopening: false };
  const state = componentFunctions(component, ['submit', 'send', 'reopen'], {
    sessionId: 'A', ownerId: 'A', draft, closed: true, running: false,
    parseInteractiveSlashInput: () => null,
    runtimeDrafts: { owns: () => current },
    runtimeThreads: {
      reopen: () => new Promise<void>((resolve) => { finish = resolve; }),
      prompt: async (id: string) => { calls.push(id); return { sequence: 1 }; },
    },
    toastError: () => {}, ApiError: class extends Error {},
  });
  state.submit();
  current = false;
  finish();
  await new Promise((resolve) => setTimeout(resolve, 0));
  assert.deepEqual(calls, []);
});
