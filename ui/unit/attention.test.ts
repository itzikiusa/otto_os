// The "needs you" bell as a raise/lower flag (lib/attention.ts) and the
// workspace store that persists it: raised by a turn end / prompt (the
// daemon's `:waiting` notice), lowered as soon as the operator enters or
// focuses the session, never re-raised by a reload, raised again by the NEXT
// turn end.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import {
  emptyAttention, flagged, flaggedIds, forget, mergeAttention, noticeIsNew, parseAttention, raise, see, serializeAttention,
} from '../src/lib/attention.ts';
import { loadSource } from './sourceHarness.ts';
import * as sessionScope from '../src/lib/stores/sessionScope.ts';
import * as sessionPatch from '../src/lib/stores/sessionPatch.ts';
import * as sessionBuckets from '../src/lib/stores/sessionBuckets.ts';

test('raise on turn end, lower on focus, re-raise on the next turn end', () => {
  let a = emptyAttention();
  assert.equal(flagged(a, 's1'), false, 'idle alone is never a flag');
  a = raise(a, 's1', 1000);
  assert.equal(flagged(a, 's1'), true);
  a = see(a, 's1', 1500); // entered / focused, did nothing
  assert.equal(flagged(a, 's1'), false);
  a = see(a, 's1', 1700);
  assert.equal(flagged(a, 's1'), false, 'stays down');
  a = raise(a, 's1', 2000); // next turn end
  assert.equal(flagged(a, 's1'), true);
  assert.deepEqual(flaggedIds(a), ['s1']);
});

test('a raise while the operator watches the session is seen at once', () => {
  const a = raise(emptyAttention(), 's1', 1000, true);
  assert.equal(flagged(a, 's1'), false);
});

test('no re-raise on reload: an answered notice (daemon clock) stays answered', () => {
  let a = raise(emptyAttention(), 's1', 1000);
  a = see(a, 's1', 1500, /* newest notice, daemon clock */ 9_000);
  const reloaded = parseAttention(serializeAttention(a));
  assert.equal(flagged(reloaded, 's1'), false);
  assert.equal(noticeIsNew(reloaded, 's1', 9_000), false, 'the answered notice');
  assert.equal(noticeIsNew(reloaded, 's1', 8_000), false, 'an older one');
  assert.equal(noticeIsNew(reloaded, 's1', 9_001), true, 'a newer one re-raises');
  // An unseen flag survives the reload.
  const unseen = parseAttention(serializeAttention(raise(emptyAttention(), 's2', 900)));
  assert.equal(flagged(unseen, 's2'), true);
});

test('clocks never mix: a skewed daemon time cannot pin or hide a flag', () => {
  // The daemon clock runs far ahead of this client: its notice time only
  // feeds the notice map; raise / seen use the client clock.
  let a = raise(emptyAttention(), 's1', 1000, false, 9e15);
  assert.equal(flagged(a, 's1'), true);
  a = see(a, 's1', 1100, 9e15);
  assert.equal(flagged(a, 's1'), false, 'entering lowers it despite the skew');
  a = raise(a, 's1', 1200, false, 9e15 + 1);
  assert.equal(flagged(a, 's1'), true, 'the next turn end raises it');
  // …and far behind: an old-looking notice time doesn't stop a live raise.
  assert.equal(flagged(raise(see(emptyAttention(), 's2', 5000), 's2', 6000, false, 1), 's2'), true);
});

test('raises never move backwards; windows merge to the latest of each', () => {
  let a = raise(emptyAttention(), 's1', 2000);
  a = raise(a, 's1', 1000);
  assert.equal(a.raised.s1, 2000);
  const other = see(raise(emptyAttention(), 's3', 50), 's1', 2500);
  const m = mergeAttention(a, other);
  assert.equal(flagged(m, 's1'), false);
  assert.equal(flagged(m, 's3'), true);
});

test('storage is bounded and tolerant; forget drops a session', () => {
  assert.deepEqual(parseAttention('nope'), emptyAttention());
  assert.deepEqual(parseAttention('{"raised":{"a":1,"b":"x"},"seen":[1]}'), { raised: { a: 1 }, seen: {}, notice: {} });
  let a = emptyAttention();
  for (let i = 0; i < 10; i++) a = raise(a, `s${i}`, i);
  const kept = parseAttention(serializeAttention(a, 3));
  assert.deepEqual(Object.keys(kept.raised).sort(), ['s7', 's8', 's9']);
  assert.equal(flagged(forget(a, 's9'), 's9'), false);
});

// ── the workspace store ───────────────────────────────────────────────────
function store(storage: Map<string, string>, module = 'agents') {
  const { ws } = loadSource(new URL('../src/lib/stores/workspace.svelte.ts', import.meta.url), {
    '../api/client': { api: {}, getToken: () => 'tok' }, './workflowRuns.svelte': { workflowRuns: { list: [], refresh: async () => {}, apply() {}, clear() {} } }, '../api/workspaces': { fetchWorkspace: async () => ({}) },
    '../router.svelte': { router: { module, parts: [module], go() {} } }, '../toast.svelte': { toasts: { info() {}, success() {}, error() {}, push() { return 1; } } }, '../toastError': { toastError: () => {} }, '../plural': { plural: (n: number, w: string) => `${n} ${w}` }, '../confirm.svelte': { confirmer: {} },
    './ui.svelte': { ui: { sessionIsolation: false, closeTabPref: '', setCloseTabPref() {} }, clientId: () => 'test' },
    '../win': { winKey: (key: string) => key }, './splitLayout.svelte': { layout: { panes: [], focusedIndex: 0, bindKey() {}, restore() {}, retain() {}, replaceSession() {}, persist() {}, removeSession() {}, closeSession() {} } }, './splitLayout': { MAX_PANES: 15, LS_PANES: 'otto_panes_' },
    '../storage': { lsGet: (k: string) => storage.get(k) ?? null, lsSet: (k: string, v: string) => void storage.set(k, v), lsRemove: (k: string) => void storage.delete(k) },
    '../desktop': { isEmbedded: false }, './sessionScope': sessionScope, './sessionPatch': sessionPatch, './sessionBuckets': sessionBuckets,
    '../lazy-component.svelte': { whenIdle: () => () => {} },
  });
  return ws;
}

test('store: a background session is flagged; focusing it lowers; a reload keeps it down', () => {
  const storage = new Map<string, string>();
  const ws = store(storage);
  ws.activeSessionId = 'other';
  ws.markNeedsYou('s1', { noticeAt: 5_000 });
  assert.equal(ws.needsYou.s1, true);
  assert.equal(ws.unread.s1, true);
  // The operator focuses it (App's effect: clearNeedsYou + the newest notice time).
  ws.activeSessionId = 's1';
  ws.clearNeedsYou('s1', 5_000);
  assert.equal(ws.needsYou.s1, undefined);
  // Reload: a fresh store reads the same storage; restoreNeedsYou re-derives
  // from the still-unread notice it was raised by — it stays down.
  const again = store(storage);
  assert.equal(again.needsYou.s1, undefined);
  again.markNeedsYou('s1', { noticeAt: 5_000, restore: true });
  assert.equal(again.needsYou.s1, undefined);
  // The next turn end raises it again.
  again.activeSessionId = 'other';
  again.markNeedsYou('s1');
  assert.equal(again.needsYou.s1, true);
  // …and an unseen flag survives a reload.
  assert.equal(store(storage).needsYou.s1, true);
});

test('store: a prompt in the session being watched never raises; away from the window it does', () => {
  const ws = store(new Map());
  ws.activeSessionId = 's1';
  ws.markNeedsYou('s1');
  assert.equal(ws.needsYou.s1, undefined, 'watching it');
  ws.operatorPresent = false; // window in the background
  ws.markNeedsYou('s1');
  assert.equal(ws.needsYou.s1, true);
  // On another page the focused pane isn't "watched" either.
  const off = store(new Map(), 'git');
  off.activeSessionId = 's1';
  off.markNeedsYou('s1');
  assert.equal(off.needsYou.s1, true);
});

test('store: the daemon\'s session_attention event raises the flag (no notice needed)', () => {
  const ws = store(new Map());
  ws.activeSessionId = 'other';
  ws.applyEvent({ type: 'session_attention', session_id: 's1', workspace_id: 'w', reason: 'turn_end' });
  assert.equal(ws.needsYou.s1, true);
});

test('store: a restored flag settles once its status is known; a fresh raise while working stays', () => {
  const storage = new Map<string, string>();
  const first = store(storage);
  first.activeSessionId = 'other';
  first.markNeedsYou('a');
  first.markNeedsYou('b');
  const ws = store(storage);
  assert.deepEqual([...ws.restoredFlagIds()].sort(), ['a', 'b']);
  ws.settleRestoredFlag('a', 'working'); // resumed while we were away: stale
  ws.settleRestoredFlag('b', 'idle'); // still waiting: keep
  assert.equal(ws.needsYou.a, undefined);
  assert.equal(ws.needsYou.b, true);
  assert.deepEqual([...ws.restoredFlagIds()], []);
  // A prompt raised in THIS run while the status still reads working.
  ws.activeSessionId = 'other';
  ws.markNeedsYou('c');
  ws.settleRestoredFlag('c', 'working');
  assert.equal(ws.needsYou.c, true, 'not a restored flag: untouched');
});

test('store: a reload re-raises only from a notice newer than the one answered', () => {
  const storage = new Map<string, string>();
  const ws = store(storage);
  ws.activeSessionId = 'other';
  ws.markNeedsYou('s1', { noticeAt: 100 });
  ws.clearNeedsYou('s1', 100);
  const again = store(storage);
  again.activeSessionId = 'other';
  again.markNeedsYou('s1', { noticeAt: 100, restore: true });
  assert.equal(again.needsYou.s1, undefined);
  again.markNeedsYou('s1', { noticeAt: 101, restore: true });
  assert.equal(again.needsYou.s1, true);
});
