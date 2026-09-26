// Router leave-guards (router.guard): go(), back/forward and a direct hash
// change all await every guard; a false keeps the route (a hash change is
// reverted to the previous hash); replace() is never guarded.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { loadSource } from './sourceHarness.ts';

const flush = async () => {
  for (let i = 0; i < 20; i++) await Promise.resolve();
};

function fixture() {
  const listeners: (() => void)[] = [];
  const location = { hash: '#/home' };
  const fire = () => listeners.forEach((l) => l());
  const window = {
    location: new Proxy(location, {
      set(t, k, v) {
        const prev = t.hash;
        (t as Record<string, unknown>)[k as string] = v;
        // A real browser fires hashchange asynchronously on a change.
        if (k === 'hash' && v !== prev) queueMicrotask(fire);
        return true;
      },
    }),
    addEventListener: (type: string, fn: () => void) => {
      if (type === 'hashchange') listeners.push(fn);
    },
  };
  const history = {
    replaceState: (_s: unknown, _t: string, h: string) => {
      location.hash = h;
    },
  };
  const { router } = loadSource(
    new URL('../src/lib/router.svelte.ts', import.meta.url),
    {
      // Like this harness's identity runes, the map models routing state only.
      // Same-session token reactivity is exercised by the browser access suite.
      'svelte/reactivity': { SvelteMap: Map },
      './win': { winKey: (k: string) => k },
      './storage': { lsGet: () => null, lsSet: () => {} },
      './desktop': { isEmbedded: false },
    },
    { window, history },
  );
  /** Simulate a link / pasted hash (bypasses go()). */
  const typeHash = async (h: string) => {
    location.hash = h;
    fire();
    await flush();
  };
  return { router, location, typeHash };
}

test('go() without guards navigates', async () => {
  const f = fixture();
  f.router.go('git');
  await flush();
  assert.equal(f.location.hash, '#/git');
  assert.equal(f.router.module, 'git');
});

test('a guard returning false keeps the route; true lets it through', async () => {
  const f = fixture();
  let allow = false;
  const seen: string[] = [];
  const off = f.router.guard(async (to: string) => {
    seen.push(to);
    return allow;
  });
  f.router.go('git');
  await flush();
  assert.equal(f.location.hash, '#/home');
  assert.deepEqual(seen, ['git']);
  allow = true;
  f.router.go('git');
  await flush();
  assert.equal(f.location.hash, '#/git');
  assert.equal(f.router.module, 'git');
  off();
});

test('a link / typed hash is reverted when a guard declines', async () => {
  const f = fixture();
  f.router.guard(() => false);
  await f.typeHash('#/vault');
  assert.equal(f.location.hash, '#/home');
  assert.equal(f.router.module, 'home');
});

test('back() asks the guards; unregistering removes the guard', async () => {
  const f = fixture();
  f.router.go('git');
  await flush();
  const off = f.router.guard(() => false);
  f.router.back();
  await flush();
  assert.equal(f.location.hash, '#/git');
  off();
  f.router.back();
  await flush();
  assert.equal(f.location.hash, '#/home');
});

test('replace() is never guarded', async () => {
  const f = fixture();
  f.router.guard(() => false);
  f.router.replace('home/overview');
  assert.equal(f.location.hash, '#/home/overview');
});
