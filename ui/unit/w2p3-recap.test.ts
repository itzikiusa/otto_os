// ux-design-6w1-01: the room recap panel says WHAT failed before the cause.
// A failed "Generate summary" leads with "Couldn’t start the summary." and the
// human cause (`loadErrorText`), never the daemon's raw text alone; a failed
// background refresh over a loaded recap keeps the recap and hands the error to
// LoadState's stale bar, which reads "Couldn’t refresh this recap. …".
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { componentFunctions } from './componentFunctions.ts';
import { loadErrorText } from '../src/lib/loadError.ts';

const panel = new URL('../src/modules/rooms/RecapPanel.svelte', import.meta.url);

function recap(request: (path: string) => Promise<unknown>) {
  const loads: boolean[] = [];
  return componentFunctions(panel, ['generate', 'poll', 'readRevision', 'sameBody'], {
    recapId: 'r1',
    confirmer: { ask: async () => true },
    recapRequest: request,
    loadErrorText,
    load: async (showLoading = true) => { loads.push(showLoading); return true; },
    loads,
    busy: false,
    actionError: '',
    tab: 'transcript',
    error: '',
    live: true,
    request: 0,
    loadingBody: false,
    after: 0,
    loadedAfter: 0,
    revision: { events_revision: 1, draft_revision: 1, metadata: {} },
    detail: { metadata: {}, next_cursor: null, events: [] },
  });
}

test('a failed summary start leads with what failed, then the cause', async () => {
  const state = recap(async () => { throw new Error('Codex CLI is signed out.'); });
  await state.generate();
  assert.equal(state.actionError, 'Couldn’t start the summary. Codex CLI is signed out.');
  assert.equal(state.busy, false);
  assert.equal(state.tab, 'transcript', 'a failed start does not switch to the summary tab');
});

test('a status error is described in the user’s terms, not as a raw status', async () => {
  const state = recap(async () => { throw Object.assign(new Error(''), { status: 401 }); });
  await state.generate();
  assert.equal(state.actionError, 'Couldn’t start the summary. Your session expired. Sign in again, then retry.');
});

test('a successful start clears the previous failure and opens the summary', async () => {
  const state = recap(async () => ({}));
  state.actionError = 'Couldn’t start the summary. Earlier failure.';
  await state.generate();
  assert.equal(state.actionError, '');
  assert.equal(state.tab, 'summary');
  assert.deepEqual(state.loads, [false]);
});

test('a failed refresh keeps the loaded recap, so LoadState shows its stale bar', async () => {
  const state = recap(async () => { throw new TypeError('fetch failed'); });
  const before = state.detail;
  assert.equal(await state.poll(), false);
  assert.equal(state.error, 'Otto can’t reach the daemon.');
  assert.equal(state.detail, before, 'the recap stays on screen (empty={!detail} is false)');
  // The stale bar's spoken line, from the shared LoadState.
  const loadState = componentFunctions(new URL('../src/lib/components/LoadState.svelte', import.meta.url),
    ['staleAnnouncement'], {});
  assert.equal(loadState.staleAnnouncement('this recap', state.error, !state.detail),
    'Couldn’t refresh this recap. Otto can’t reach the daemon.');
});

// The same rule in the start-room sheet: a failed native window says the room
// window is what failed (the room exists; Retry reopens it), a failed creation
// says the room didn't start — never the raw cause alone.
function startSheet(post: () => Promise<unknown>, open: () => Promise<void>) {
  const closed: boolean[] = [];
  const state = componentFunctions(new URL('../src/modules/rooms/StartRoomModal.svelte', import.meta.url), ['start'], {
    sessionId: 's1', name: 'Ada', busy: false, error: '', created: null, loadErrorText,
    api: { post }, baseUrl: () => 'http://daemon', rememberRoom: () => {}, openHostedRoom: open,
    onclose: () => closed.push(true),
  });
  return { state, closed };
}

test('a failed room window leads with “Couldn’t open the room window.”', async () => {
  const { state, closed } = startSheet(async () => ({ room_id: 'r1' }), async () => { throw new Error('Native window refused.'); });
  await state.start();
  assert.equal(state.error, 'Couldn’t open the room window. Native window refused.');
  assert.deepEqual(state.created, { room_id: 'r1' }, 'the created room is kept for Retry');
  assert.equal(closed.length, 0);
});

test('a failed room creation leads with “Couldn’t start this room.”', async () => {
  const { state } = startSheet(async () => { throw new TypeError('fetch failed'); }, async () => {});
  await state.start();
  assert.equal(state.error, 'Couldn’t start this room. Otto can’t reach the daemon.');
  assert.equal(state.created, null);
});
