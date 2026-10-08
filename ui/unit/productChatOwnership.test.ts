import { test } from 'node:test';
import assert from 'node:assert/strict';
import { componentFunctions } from './componentFunctions.ts';
import { deferred } from './sourceHarness.ts';

for (const [file, idKey, load, get, send] of [
  ['DiscoveryChat', 'cid', 'loadChat', 'getDiscoveryChat', 'sendDiscoveryMessage'],
  ['RefineChat', 'tid', 'loadThread', 'getRefinementThread', 'sendRefinementMessage'],
]) {
  function fixture() {
    const read = deferred<any>(), write = deferred<any>();
    const state = componentFunctions(new URL(`../src/modules/product/${file}.svelte`, import.meta.url), [load, 'send'], {
      [idKey]: 'A', activeId: 'A', loadSeq: 0, drafts: new Map(), alive: true,
      inputText: 'Draft A', messages: [], sending: false, loading: false, loadError: null,
      provider: 'claude', threadModel: null, chatModel: null,
      product: {
        [get]: (id: string) => id === 'A' ? read.promise : Promise.resolve({ messages: [{ id: 'B-message' }], chat: { model: null }, thread: { model: null } }),
        [send]: () => write.promise,
      }, loadErrorText: String, toastError() {},
    });
    return { state, read, write };
  }
  test(`${file} late load cannot replace the newly selected conversation`, async () => {
    const { state, read } = fixture();
    const pending = state[load]('A'); state[idKey] = 'B'; await state[load]('B');
    read.resolve({ messages: [{ id: 'A-message' }], chat: { model: null }, thread: { model: null } }); await pending;
    assert.equal(state.messages[0].id, 'B-message');
  });
  test(`${file} late failed send preserves both conversations' drafts`, async () => {
    const { state, write } = fixture();
    const pending = state.send(); state[idKey] = 'B'; await state[load]('B');
    state.inputText = 'Draft B'; write.reject(new Error('send failed')); await pending;
    assert.equal(state.inputText, 'Draft B');
    assert.equal(state.messages[0].id, 'B-message');
    assert.equal(state.drafts.get('A'), 'Draft A');
  });
  test(`${file} late successful send cannot append A messages to B`, async () => {
    const { state, write } = fixture();
    const pending = state.send(); state[idKey] = 'B'; await state[load]('B');
    write.resolve({ user_message: { id: 'A-user' }, agent_message: { id: 'A-agent' } }); await pending;
    assert.deepEqual(Array.from(state.messages, (x: any) => x.id), ['B-message']);
  });
}
