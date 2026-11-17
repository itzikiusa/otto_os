// IDE collab, round 4 daemon rules: `peer_left` drops the peer's cursor; event rooms (guest
// view) never clobber the host's room; a 409 "not keeping up" resyncs and resends the
// guest's own text; a refused protected save (403) and a refused Restart are said inline.
import assert from 'node:assert/strict';
import { test } from 'node:test';
import { loadSource } from './sourceHarness.ts';

const json = (status: number, body: unknown) => new Response(JSON.stringify(body), { status, headers: { 'Content-Type': 'application/json' } });

function collabModules(laneFetch: (path: string, init?: RequestInit) => Promise<Response>) {
  const toastLog: string[] = [];
  const cache = new Map();
  const imports = {
    svelte: { tick: async () => {} },
    '../../../lib/api/client': { api: {}, ApiError: class extends Error { status = 0; }, getToken: () => '', laneFetch: (_lane: string, p: string, init?: RequestInit) => laneFetch(p, init) },
    '../../../lib/components/cm-ide-drafts': { readIdeUndoDraft: () => null, removeIdeUndoDraft: () => {}, writeIdeUndoDraftImmediate: () => {} },
    '../../../lib/components/cm-ide-prefs': { readIdePreferences: () => ({ autoSave: false, localHistoryEnabled: false }) },
    '../../../lib/confirm.svelte': { confirmer: { ask: async () => true, choose: async () => ({ value: null }) } },
    '../../../lib/toast.svelte': { toasts: { success: (t: string) => toastLog.push(`ok:${t}`), error: (t: string) => toastLog.push(`err:${t}`), info: () => {} } },
    '../../../lib/toastError': { toastError: (t: string) => toastLog.push(`err:${t}`) },
    '../../../lib/ideProjectsModel': { loadNamedIdeProjectsLocal: () => [], saveNamedIdeProjectsLocal: () => {}, fetchNamedIdeProjectsFromServer: async () => [], parseNamedIdeProject: () => null },
    '../../../lib/ideWorkspacePort.svelte': { ideWorkspacePort: { rootsForLocal: () => [] } },
    '../../../lib/storage': { lsGet: () => null, lsSet: () => {}, lsRemove: () => {} },
  };
  const opts = { relative: true };
  const lsp = loadSource(new URL('../src/modules/ide/shell/ideLspAndSearchHelpers.ts', import.meta.url), imports, {}, opts, cache);
  const sync = loadSource(new URL('../src/modules/ide/store/ideCollabSync.ts', import.meta.url), imports, {}, opts, cache);
  const notices = loadSource(new URL('../src/modules/ide/store/ideCollabNotices.svelte.ts', import.meta.url), imports, {}, opts, cache).ideCollabNotices;
  const types = loadSource(new URL('../src/modules/ide/store/ideTypes.ts', import.meta.url), imports, {}, opts, cache);
  return { lsp, sync, notices, types, toastLog };
}

function guestHost(buffer: string) {
  const live = { text: buffer };
  return {
    live,
    target: { kind: 'local', root: '/repo' }, targetPayload: { kind: 'local', root: '/repo' },
    tabs: [{ path: '/repo/a.ts', name: 'a.ts', content: buffer, savedContent: buffer, eol: '\n', loading: false, error: null, readOnly: false, truncated: false }],
    activePath: '/repo/a.ts', canEditCollab: true, collabIsGuest: true,
    collabRoom: { id: 'r1', title: 'room', target: { kind: 'local', root: '/repo' }, default_role: 'editor', peers: [] },
    collabPeerId: 'p1', collabPeerSecret: 's', collabGuestToken: 't',
    sharedBuffers: { '/repo/a.ts': { content: buffer, version: 3 } } as Record<string, { content: string; version: number }>,
    resolvePath: (p: string) => p, getBuffer: () => live.text,
  };
}

test('collab: a 403 on a guest save of a protected file is kept as that file\'s inline reason, not a toast', async () => {
  const { lsp, buf, notices, toastLog } = { ...collabModules(async () => json(403, { error: 'agent instruction files are host-only' })), buf: null };
  void buf;
  const perf = { flush: () => '' };
  const h = guestHost('x');
  assert.equal(await lsp.runSendCollabEdit(h, perf, '/repo/a.ts', 'x', true), false);
  assert.equal(notices.saveRefusedByPath['/repo/a.ts'], 'agent instruction files are host-only');
  assert.deepEqual(toastLog, []);
});

test('collab: a 409 "not keeping up" resyncs and resends the guest\'s current text', async () => {
  const sent: { content: string; base_version?: number }[] = [];
  let edits = 0;
  const { lsp, sync, notices } = collabModules(async (p, init) => {
    if (p.endsWith('/state')) return json(200, { room: { id: 'r1', title: 'room', target: { kind: 'local', root: '/repo' }, default_role: 'editor', peers: [] }, shared_buffers: { '/repo/a.ts': { content: 'theirs', version: 9 } } });
    const body = JSON.parse(String(init?.body));
    sent.push(body);
    edits += 1;
    return edits === 1 ? json(409, { error: "a collaborator's connection is not keeping up with edits; try again shortly" }) : json(200, { version: 10 });
  });
  // No real waiting in the test: the delays are replaced through recoverRefusedEdit's `wait`.
  const realRecover = sync.recoverRefusedEdit;
  sync.recoverRefusedEdit = (host: unknown, resend: () => Promise<void>) => realRecover(host, resend, async () => {});
  const h = guestHost('mine v1');
  const pending = lsp.runSendCollabEdit(h, { flush: () => '' }, '/repo/a.ts', 'mine v1', false);
  h.live.text = 'mine v2'; // kept typing while refused
  const ok = await pending;
  sync.recoverRefusedEdit = realRecover;
  assert.equal(ok, true);
  assert.equal(sent.length, 2);
  assert.equal(sent[1].content, 'mine v2', 'the guest\'s own latest text is resent, never the room copy');
  assert.equal(sent[1].base_version, 9, 'with the version re-read from the room');
  assert.equal(h.sharedBuffers['/repo/a.ts'].version, 10);
  assert.equal(notices.roomSync, null);
});

test('collab: recovery is bounded and ends in a calm "failed" state', async () => {
  const { sync, notices, types } = collabModules(async () => json(200, { shared_buffers: {} }));
  const h = guestHost('x');
  let tries = 0;
  const ok = await sync.recoverRefusedEdit(h, async () => { tries += 1; throw new types.IdeCollabHttpError(409, 'not keeping up'); }, async () => {});
  assert.equal(ok, false);
  assert.equal(tries, sync.COLLAB_RESYNC_DELAYS_MS.length);
  assert.equal(notices.roomSync, 'failed');
});

test('collab: peer_left drops the peer\'s cursor; an event room never clobbers the host\'s own room', () => {
  let onEvent: ((raw: string) => void) | null = null;
  const ctl = loadSource(new URL('../src/modules/ide/store/ideCollabController.ts', import.meta.url), {
    '../../../lib/api/client': { api: {}, laneFetch: async () => json(200, {}) },
    '../../../lib/toast.svelte': { toasts: { success() {}, error() {}, info() {} } },
    '../../../lib/toastError': { toastError() {} },
    './ideCollabNotices.svelte': { ideCollabNotices: { reset() {} } },
    './ideCollabSync': { mergeText: (_b: string, _m: string, theirs: string) => theirs },
    './ideTypes': { collabGuestPost: async () => ({}), ideBasename: (p: string) => p, streamIdeSse: (_p: string, _h: unknown, _s: unknown, cb: (raw: string) => void) => { onEvent = cb; return Promise.resolve(); } },
  });
  const hostRoom = { id: 'r1', title: 'Pairing', target: { kind: 'local', root: '/repo', workspace_id: 'w1' }, default_role: 'viewer', peers: [{ peer_id: 'h' }, { peer_id: 'g1' }] };
  const store = {
    collabIsGuest: false, collabRoom: hostRoom, followingPeerId: 'g1',
    peerCursors: [{ peer_id: 'g1' }, { peer_id: 'g2' }], tabs: [], sharedBuffers: {},
  } as Record<string, unknown>;
  ctl.attachCollabEventStream(store, 'r1', 'tok', 'h', null);
  onEvent!(JSON.stringify({ type: 'peer_left', peer_id: 'g1', room: { id: 'r1', title: 'Pairing', target: { kind: 'local', root: '/repo' }, default_role: 'viewer', peers: [{ peer_id: 'h' }] } }));
  assert.deepEqual((store.peerCursors as { peer_id: string }[]).map((p) => p.peer_id), ['g2']);
  assert.equal(store.followingPeerId, null);
  const room = store.collabRoom as typeof hostRoom;
  assert.equal(room.peers.length, 1, 'the peer list refreshes from the event');
  assert.equal(room.target.workspace_id, 'w1', 'the host keeps its own target, not the stripped guest view');
});

test('format: a deliberate "skipped" answer leaves the buffer alone, is no error, and says so once per file', async () => {
  const notes: string[] = [];
  const cache = new Map();
  const imports = {
    svelte: { tick: async () => {} },
    '../../../lib/api/client': {
      api: { post: async () => ({ formatted: 'const a=1', changed: false, formatter: "Formatting skipped: this project's Prettier config runs code. Allow it in the project's environment settings (OTTO_IDE_TRUST_FORMATTER_CONFIG=1)" }) },
      ApiError: class extends Error { status = 0; }, getToken: () => '', laneFetch: async () => json(200, {}),
    },
    '../../../lib/components/cm-ide-drafts': { readIdeUndoDraft: () => null, removeIdeUndoDraft: () => {}, writeIdeUndoDraftImmediate: () => {} },
    '../../../lib/components/cm-ide-prefs': { readIdePreferences: () => ({}) },
    '../../../lib/confirm.svelte': { confirmer: {} },
    '../../../lib/toast.svelte': { toasts: { success: (t: string) => notes.push(`ok:${t}`), error: (t: string) => notes.push(`err:${t}`), info: (t: string, b?: string) => notes.push(`info:${t}|${b ?? ''}`) } },
    '../../../lib/toastError': { toastError: (t: string) => notes.push(`err:${t}`) },
    '../../../lib/ideProjectsModel': { loadNamedIdeProjectsLocal: () => [], saveNamedIdeProjectsLocal: () => {}, fetchNamedIdeProjectsFromServer: async () => [], parseNamedIdeProject: () => null },
    '../../../lib/ideWorkspacePort.svelte': { ideWorkspacePort: {} },
    '../../../lib/storage': { lsGet: () => null, lsSet: () => {}, lsRemove: () => {} },
  };
  const lsp = loadSource(new URL('../src/modules/ide/shell/ideLspAndSearchHelpers.ts', import.meta.url), imports, {}, { relative: true }, cache);
  let edits = 0;
  const tab = { path: '/repo/a.ts', name: 'a.ts', language: 'typescript', readOnly: false, truncated: false };
  const host = { activeTab: tab, tabs: [tab], targetPayload: {}, applyAgentEdit: async () => { edits += 1; return true; } };
  const perf = { flush: () => 'const a = 1', getBuffer: () => 'const a = 1' };
  await lsp.runFormatActiveFile(host, perf);
  await lsp.runFormatActiveFile(host, perf);
  assert.equal(edits, 0, 'the buffer is left untouched');
  assert.deepEqual(notes, ["info:Formatting skipped: this project's Prettier config runs code.|Allow it in the project's environment settings (OTTO_IDE_TRUST_FORMATTER_CONFIG=1)"], 'one quiet notice with its hint, no error');
});
