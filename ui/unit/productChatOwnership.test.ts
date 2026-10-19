import { test } from 'node:test';
import assert from 'node:assert/strict';
import { componentFunctions } from './componentFunctions.ts';
import { deferred } from './sourceHarness.ts';

for (const [file, idKey, load, get, send, stop] of [
  ['DiscoveryChat', 'cid', 'loadChat', 'getDiscoveryChat', 'sendDiscoveryMessage', 'stopDiscoveryTurn'],
  ['RefineChat', 'tid', 'loadThread', 'getRefinementThread', 'sendRefinementMessage', 'stopRefinementTurn'],
]) {
  function fixture() {
    const read = deferred<any>(), write = deferred<any>(), stopReq = deferred<any>();
    const sentTurns: Array<{ id: string; body: string; turnId?: string }> = [];
    const stoppedTurns: Array<{ id: string; turnId: string }> = [];
    let reloadMessages: any[] = [];
    const state = componentFunctions(
      new URL(`../src/modules/product/${file}.svelte`, import.meta.url),
      [load, 'send', 'stopTurn', 'stopWaiting'],
      {
        [idKey]: 'A',
        activeId: 'A',
        loadSeq: 0,
        drafts: new Map(),
        stopRequestedTurns: new Set(),
        alive: true,
        inFlightTurnId: null,
        inputText: 'Draft A',
        messages: [],
        sending: false,
        stopping: false,
        activeTurnId: null,
        stopNotice: null,
        backgroundTurn: false,
         loading: false,
        loadError: null,
        provider: 'claude',
        threadModel: null,
        chatModel: null,
        newTurnId: () => `turn-${sentTurns.length + 1}`,
        product: {
          [get]: (id: string) =>
            id === 'A'
              ? read.promise
              : Promise.resolve({
                  messages: id === 'A-reload' ? reloadMessages : [{ id: 'B-message' }],
                  chat: { model: null },
                  thread: { model: null },
                }),
          [send]: (id: string, body: string, _provider?: string, _model?: string, turnId?: string) => {
            sentTurns.push({ id, body, turnId });
            return write.promise;
          },
          [stop]: (id: string, turnId: string) => {
            stoppedTurns.push({ id, turnId });
            return stopReq.promise;
          },
        },
        loadErrorText: String,
        toastError() {},
      },
    );
    return {
      state,
      read,
      write,
      stopReq,
      sentTurns,
      stoppedTurns,
      setReloadMessages(msgs: any[]) {
        reloadMessages = msgs;
      },
    };
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
  test(`${file} stopTurn sends matching turn_id, holds Stopping… until ack, reloads on stopped:true, and never restores accepted user message as draft`, async () => {
    const { state, read, write, stopReq, sentTurns, stoppedTurns } = fixture();
    const pendingSend = state.send();
    assert.equal(sentTurns.length, 1);
    assert.equal(sentTurns[0].turnId, 'turn-1');
    assert.equal(state.sending, true);
    assert.equal(state.activeTurnId, 'turn-1');
    assert.equal(state.inputText, '');

    const pendingStop = state.stopTurn();
    assert.equal(state.stopping, true);
    assert.deepEqual(stoppedTurns, [{ id: 'A', turnId: 'turn-1' }]);

    // Server kills the turn so POST /messages rejects while /stop is completing cleanup.
    write.reject(new Error('turn cancelled'));
    await pendingSend;
    // While /stop is still awaiting ack, stopping remains true and accepted user bubble is not rolled back.
    assert.equal(state.stopping, true);
    assert.equal(state.inputText, '');
    assert.equal(state.drafts.has('A'), false);
    assert.equal(state.messages.length, 1);
    assert.equal(state.messages[0].body, 'Draft A');

    // /stop resolves { stopped: true } and triggers a reload of conversation A.
    read.resolve({
      messages: [{ id: 'persisted-user-1', role: 'user', body: 'Draft A' }],
      chat: { model: null },
      thread: { model: null },
    });
    stopReq.resolve({ stopped: true });
    await pendingStop;

    assert.equal(state.stopping, false);
    assert.equal(state.sending, false);
    assert.equal(state.activeTurnId, null);
    assert.equal(state.stopNotice, 'Turn stopped.');
    assert.equal(state.inputText, '');
    assert.equal(state.drafts.has('A'), false);
    assert.deepEqual(Array.from(state.messages, (x: any) => x.id), ['persisted-user-1']);
  });
  test(`${file} stopTurn with stopped:false does not claim cancellation while send remains in flight, and switching chats never restores stopped turn as draft`, async () => {
    const { state, write, stopReq, stoppedTurns } = fixture();
    const pendingSend = state.send();
    const pendingStop = state.stopTurn();
    assert.deepEqual(stoppedTurns, [{ id: 'A', turnId: 'turn-1' }]);

    stopReq.resolve({ stopped: false });
    await pendingStop;
    // stopped: false is not cancellation proof; turn is still in flight so sending stays true and notice is truthful.
    assert.equal(state.stopping, false);
    assert.equal(state.sending, true);
    assert.match(String(state.stopNotice), /Couldn’t confirm stop/);

    // Switch to conversation B and let the old turn fail late; A must not regain a duplicate draft.
    state[idKey] = 'B';
    await state[load]('B');
    write.reject(new Error('late abort'));
    await pendingSend;
    assert.equal(state.drafts.has('A'), false);
  });
}
