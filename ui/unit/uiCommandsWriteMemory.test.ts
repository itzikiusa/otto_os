import { test } from 'node:test';
import assert from 'node:assert/strict';
import { loadSource } from './sourceHarness.ts';
import { WRITES_KEY, WriteMemory, writeScope } from '../src/lib/uiCommands/writeMemory.ts';

// "Allow writes on ‹connection› for this session": confirmWrite (lib/uiCommands.ts)
// skips the dialog when `writeScope(opts)` is non-null AND the uiControl store
// remembers that (session, scope); it offers the checkbox only for a non-null
// scope. These tests drive exactly those two calls, against the REAL store with
// an in-memory localStorage behind `lsGet` / `lsSet`.

function memoryStorage() {
  const map = new Map<string, string>();
  return { map, get: (k: string) => map.get(k) ?? null, set: (k: string, v: string) => void map.set(k, v) };
}

function store() {
  const storage = memoryStorage();
  const posts: { path: string; body: unknown }[] = [];
  const aborted: string[] = [];
  const sessions: { id: string; meta: Record<string, unknown> }[] = [{ id: 's1', meta: { ui_control: { enabled: true } } }];
  const ws = {
    sessions,
    otherWsSessions: [],
    panes: [],
    getSession: (id: string) => sessions.find((s) => s.id === id) ?? null,
    navigateToSession() {},
  };
  const { uiControl } = loadSource(new URL('../src/lib/stores/uiControl.svelte.ts', import.meta.url), {
    '../api/client': {
      api: {
        post: async (path: string, body: { enabled: boolean }) => {
          posts.push({ path, body });
          return { id: 's1', meta: { ui_control: { enabled: body.enabled } } };
        },
      },
    },
    '../storage': { lsGet: storage.get, lsSet: storage.set },
    '../toast.svelte': { toasts: { push() {}, info() {} } },
    '../toastError': { toastError() {} },
    './workspace.svelte': { ws },
    '../router.svelte': { router: { module: 'agents' } },
    '../desktop': { isEmbedded: false },
    '../sidebar': { moduleLabel: (m: string) => m },
    '../uiCommands/frames': loadSource(new URL('../src/lib/uiCommands/frames.ts', import.meta.url), {}),
    '../uiCommands/writeMemory': loadSource(new URL('../src/lib/uiCommands/writeMemory.ts', import.meta.url), {}),
  });
  uiControl.installAbort((sid: string) => aborted.push(sid));
  /** What confirmWrite does before opening the dialog: true = no dialog. */
  const skipsDialog = (sid: string, opts: { connId?: string; guarded?: boolean; outward?: boolean }) => {
    const scope = writeScope(opts);
    return !!scope && uiControl.writesAllowed(sid, scope);
  };
  /** The confirm was answered "Run" with the box ticked. */
  const confirmWithRemember = (sid: string, opts: { connId?: string; guarded?: boolean; outward?: boolean }) => {
    const scope = writeScope(opts);
    if (scope) uiControl.rememberWrites(sid, scope);
  };
  return { uiControl, storage, posts, aborted, skipsDialog, confirmWithRemember };
}

const DB = { connId: 'conn-staging' };

test('remembering a write makes the next one on that scope skip the dialog — and survives a reload', () => {
  const h = store();
  assert.equal(h.skipsDialog('s1', DB), false, 'the first write asks');
  h.confirmWithRemember('s1', DB);
  assert.equal(h.skipsDialog('s1', DB), true, 'the next write runs without a dialog');
  assert.ok(h.storage.map.get(WRITES_KEY)?.includes('conn-staging'));
  // A fresh store over the same storage (a reload) still remembers it.
  const again = new WriteMemory({ get: h.storage.get, set: h.storage.set });
  assert.equal(again.allowed('s1', 'conn-staging'), true);
});

test('a different scope, or another session, still asks', () => {
  const h = store();
  h.confirmWithRemember('s1', DB);
  assert.equal(h.skipsDialog('s1', { connId: 'conn-other' }), false);
  assert.equal(h.skipsDialog('s2', DB), false);
});

test('guarded and outward writes are never remembered, never skipped, never offer the checkbox', () => {
  for (const opts of [{ ...DB, guarded: true }, { ...DB, outward: true }, { connId: '   ' }, {}]) {
    assert.equal(writeScope(opts), null, JSON.stringify(opts));
  }
  const h = store();
  h.confirmWithRemember('s1', { ...DB, guarded: true });
  h.confirmWithRemember('s1', { ...DB, outward: true });
  assert.equal(h.storage.map.get(WRITES_KEY), undefined, 'nothing was stored');
  // Even with the plain scope remembered, the guarded / outward variants ask.
  h.confirmWithRemember('s1', DB);
  assert.equal(h.skipsDialog('s1', { ...DB, guarded: true }), false);
  assert.equal(h.skipsDialog('s1', { ...DB, outward: true }), false);
});

test('a revoke seen as session_meta_updated (grant off) makes it ask again; a grant-on update does not', () => {
  const h = store();
  h.confirmWithRemember('s1', DB);
  h.uiControl.applyEvent({ type: 'session_meta_updated', session_id: 's1', meta: { ui_control: { enabled: true } } });
  assert.equal(h.skipsDialog('s1', DB), true, 'a grant-on update keeps the memory');
  h.uiControl.applyEvent({ type: 'session_meta_updated', session_id: 's1', meta: { ui_control: { enabled: false } } });
  assert.equal(h.skipsDialog('s1', DB), false);
});

test('session_removed makes it ask again', () => {
  const h = store();
  h.confirmWithRemember('s1', DB);
  h.uiControl.applyEvent({ type: 'session_removed', session_id: 's1' });
  assert.equal(h.skipsDialog('s1', DB), false);
  assert.deepEqual(h.aborted, ['s1']);
});

test('setGrant(false) (Stop / toggle off) makes it ask again after a re-grant', async () => {
  const h = store();
  h.confirmWithRemember('s1', DB);
  assert.equal(await h.uiControl.setGrant('s1', false), true);
  assert.equal(h.skipsDialog('s1', DB), false);
  assert.equal(await h.uiControl.setGrant('s1', true), true);
  assert.equal(h.skipsDialog('s1', DB), false, 're-granting does not restore the old memory');
  assert.equal(JSON.stringify(h.posts.map((p) => p.body)), '[{"enabled":false},{"enabled":true}]');
});

test('a malformed stored value reads as nothing remembered', () => {
  const s = memoryStorage();
  s.map.set(WRITES_KEY, '{not json');
  const m = new WriteMemory(s);
  assert.equal(m.allowed('s1', 'x'), false);
  m.remember('s1', 'x');
  assert.equal(m.allowed('s1', 'x'), true);
  s.map.set(WRITES_KEY, JSON.stringify({ s1: 'x', s2: [1, 'y'] }));
  assert.equal(m.allowed('s1', 'x'), false, 'a non-array entry is ignored');
  assert.equal(m.allowed('s2', 'y'), true);
});
