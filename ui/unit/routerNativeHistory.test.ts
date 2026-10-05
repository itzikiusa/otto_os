// S13-09: a native traversal (mouse back, swipe, browser Back) moves the
// in-app stack pointer instead of truncating + pushing, and a refused one is
// undone with history.go — never by overwriting the back-target entry.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { loadSource } from './sourceHarness.ts';
import { captureRoomInvite } from '../src/modules/rooms/room-access.ts';
import { activeNavId } from '../src/lib/sidebar.ts';

const flush = async () => { for (let i = 0; i < 30; i++) await Promise.resolve(); };

/** A tiny browser: an entry list + pointer; hash writes push, go() traverses. */
function browser(initial = '#/home') {
  const entries: { hash: string; state: unknown }[] = [{ hash: initial, state: null }];
  let at = 0;
  const listeners: (() => void)[] = [];
  const fire = () => listeners.forEach((l) => l());
  const loc = { hash: initial };
  const location = new Proxy(loc, {
    set(t, k, v) {
      if (k === 'hash' && v !== t.hash) {
        entries.splice(at + 1);
        entries.push({ hash: v, state: null });
        at = entries.length - 1;
        t.hash = v;
        queueMicrotask(fire);
      }
      return true;
    },
  });
  const history = {
    get state() { return entries[at].state; },
    replaceState: (s: unknown, _t: string, h: string) => { entries[at] = { hash: h, state: s }; loc.hash = h; },
    go: (n: number) => {
      const next = Math.max(0, Math.min(entries.length - 1, at + n));
      if (next === at) return;
      at = next;
      loc.hash = entries[at].hash;
      queueMicrotask(fire);
    },
  };
  const window = { location, addEventListener: (t: string, fn: () => void) => { if (t === 'hashchange') listeners.push(fn); } };
  const { router } = loadSource(new URL('../src/lib/router.svelte.ts', import.meta.url), {
    'svelte/reactivity': { SvelteMap: Map },
    './win': { winKey: (k: string) => k },
    './storage': { lsGet: () => null, lsSet: () => {} },
    './desktop': { isEmbedded: false },
    '../modules/rooms/room-access': { captureRoomInvite },
    './sidebar': { activeNavId },
  }, { window, history });
  return { router, history, entries: () => entries.map((e) => e.hash), at: () => at, loc };
}

test('native Back then Forward moves the in-app pointer and keeps forward history', async () => {
  const b = browser();
  b.router.go('git'); await flush();
  b.router.go('vault'); await flush();
  b.history.go(-1); await flush(); // mouse back button
  assert.equal(b.router.module, 'git');
  assert.deepEqual(b.entries(), ['#/home', '#/git', '#/vault'], 'nothing pushed');
  b.history.go(1); await flush();
  assert.equal(b.router.module, 'vault');
  // In-app back/forward agree with where the browser is (stack intact).
  b.router.back(); await flush();
  assert.equal(b.router.module, 'git');
  b.router.back(); await flush();
  assert.equal(b.router.module, 'home');
});

test('a refused native Back is undone with history.go, not by overwriting the entry', async () => {
  const b = browser();
  b.router.go('git'); await flush();
  b.router.go('vault'); await flush();
  b.router.guard(async () => false);
  b.history.go(-1); await flush();
  assert.equal(b.loc.hash, '#/vault', 'stayed');
  assert.equal(b.router.module, 'vault');
  assert.deepEqual(b.entries(), ['#/home', '#/git', '#/vault'], 'the back target is intact');
  assert.equal(b.at(), 2);
});

test('an approved native Back under a guard lands on the target', async () => {
  const b = browser();
  b.router.go('git'); await flush();
  b.router.go('vault'); await flush();
  b.router.guard(async () => true);
  b.history.go(-1); await flush();
  assert.equal(b.router.module, 'git');
  assert.deepEqual(b.entries(), ['#/home', '#/git', '#/vault']);
  b.router.forward(); await flush();
  assert.equal(b.router.module, 'vault', 'forward history survived');
});

test('a new link navigation after Back still truncates forward history', async () => {
  const b = browser();
  b.router.go('git'); await flush();
  b.router.go('vault'); await flush();
  b.history.go(-1); await flush();
  b.router.go('settings'); await flush();
  b.router.forward(); await flush();
  assert.equal(b.router.module, 'settings', 'no forward entry left');
  b.router.back(); await flush();
  assert.equal(b.router.module, 'git');
});
