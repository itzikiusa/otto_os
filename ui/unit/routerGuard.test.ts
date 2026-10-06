// Router leave-guards (router.guard): go(), back/forward and a direct hash
// change all await every guard; a false keeps the route (a hash change is
// reverted to the previous hash); replace() is never guarded.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { loadSource } from './sourceHarness.ts';
import { captureRoomInvite, roomInvite, forgetRoom } from '../src/modules/rooms/room-access.ts';
import { activeNavId } from '../src/lib/sidebar.ts';
import { dropShareToken, storeShareToken, storedShareToken, type TokenStorage } from '../src/lib/shareTokenStore.ts';

const flush = async () => {
  for (let i = 0; i < 20; i++) await Promise.resolve();
};

/** Tab-scoped sessionStorage shared across "reloads" (fresh router loads). */
function fakeSession(): TokenStorage & { data: Map<string, string> } {
  const data = new Map<string, string>();
  return {
    data,
    getItem: (k) => data.get(k) ?? null,
    setItem: (k, v) => void data.set(k, v),
    removeItem: (k) => void data.delete(k),
  };
}
function bindShareStore(st: TokenStorage) {
  return {
    storeShareToken: (sid: string, t: string) => storeShareToken(sid, t, st),
    storedShareToken: (sid: string) => storedShareToken(sid, st),
    dropShareToken: (sid: string) => dropShareToken(sid, st),
  };
}

function fixture(initialHash = '#/home', shareStore = bindShareStore(fakeSession())) {
  const listeners: (() => void)[] = [];
  const location = { hash: initialHash };
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
      '../modules/rooms/room-access': { captureRoomInvite },
      './sidebar': { activeNavId },
      './shareTokenStore': shareStore,
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


test('room invitation is captured only in memory and removed before route history is recorded', async () => {
  const f = fixture('#/room/unit-room/invite_1234567890');
  assert.equal(f.location.hash, '#/room/unit-room');
  assert.equal(f.router.module, 'room');
  assert.equal(roomInvite('unit-room'), 'invite_1234567890');
  f.router.go('home');
  await flush();
  f.router.back();
  await flush();
  assert.equal(f.location.hash, '#/room/unit-room', 'back navigation cannot restore the capability in the URL');
  forgetRoom('unit-room');
});

test('openModule resumes a module where it was left; the active module goes home', async () => {
  const f = fixture('#/home');
  f.router.go('kubernetes/c1/pods/default/api');
  await flush();
  f.router.go('database/conn-1');
  await flush();
  // Back to Kubernetes from elsewhere → the exact route it was left on.
  f.router.openModule('kubernetes');
  await flush();
  assert.equal(f.location.hash, '#/kubernetes/c1/pods/default/api');
  // Clicking the module that is already active → its main page.
  f.router.openModule('kubernetes');
  await flush();
  assert.equal(f.location.hash, '#/kubernetes');
  // database/brokers resume under the Connections nav id.
  f.router.openModule('connections');
  await flush();
  assert.equal(f.location.hash, '#/database/conn-1');
  // A module never visited opens at its root.
  f.router.openModule('git');
  await flush();
  assert.equal(f.location.hash, '#/git');
});

test('workspace leave decisions run guards without navigating or accepting same-route exemptions', async () => {
  const f=fixture('#/api');
  let target: string | undefined;
  f.router.guard((to:string)=>{target=to;return false;});
  assert.equal(await f.router.mayChangeWorkspace(),false);
  assert.equal(target,'');assert.equal(f.location.hash,'#/api');
});

test('a newer route decision invalidates a pending workspace decision', async () => {
  const f=fixture('#/api');let release!: (value:boolean)=>void;
  const pending=new Promise<boolean>(r=>{release=r;});
  f.router.guard((to:string)=>to===''?pending:true);
  const changing=f.router.mayChangeWorkspace();f.router.go('git');
  await flush();release(true);
  assert.equal(await changing,false);assert.equal(f.location.hash,'#/git');
});

test('a share token survives a reload of the guest page (tab sessionStorage), never in the URL', async () => {
  const tab = fakeSession();
  const first = fixture('#/s/sess1/tok-123', bindShareStore(tab));
  await flush();
  assert.equal(first.location.hash, '#/s/sess1', 'token stripped from the URL');
  assert.equal(tab.data.get('otto_share:sess1'), 'tok-123');
  assert.equal(tab.data.has('otto_token'), false, 'never the owner login key');
  // Reload: a fresh router on the stripped hash, same tab storage.
  const { getShareToken } = loadReloaded('#/s/sess1', tab);
  assert.equal(getShareToken('sess1'), 'tok-123');
  assert.equal(getShareToken('other'), null);
  // A dead link forgets it — the next reload shows the no-token card.
  dropShareToken('sess1', tab);
  assert.equal(loadReloaded('#/s/sess1', tab).getShareToken('sess1'), null);
});

function loadReloaded(hash: string, tab: TokenStorage) {
  const location = { hash };
  return loadSource(
    new URL('../src/lib/router.svelte.ts', import.meta.url),
    {
      'svelte/reactivity': { SvelteMap: Map },
      './win': { winKey: (k: string) => k },
      './storage': { lsGet: () => null, lsSet: () => {} },
      './desktop': { isEmbedded: false },
      '../modules/rooms/room-access': { captureRoomInvite },
      './sidebar': { activeNavId },
      './shareTokenStore': bindShareStore(tab),
    },
    { window: { location, addEventListener: () => {}, removeEventListener: () => {} }, history: { replaceState: () => {} } },
  );
}
