// Live project tree (ideTreeLive): the lease follows what the tree shows, a daemon
// `ide_fs_changed` re-lists only folders still shown, and bursts never stack listings.
import assert from 'node:assert/strict';
import { test } from 'node:test';
import { deferred, loadSource } from './sourceHarness.ts';
import * as id from '../src/lib/id.ts';

const mod = loadSource(new URL('../src/modules/ide/store/ideTreeLive.ts', import.meta.url), {
  '../../../lib/api/client': { api: { bg: { post: async () => ({}) } } },
  '../../../lib/id': id,
});
const { IdeTreeLive, shownDirs } = mod;

function host(over: Record<string, unknown> = {}) {
  const relisted: string[] = [];
  const gates = new Map<string, ReturnType<typeof deferred<void>>>();
  const h = {
    target: { kind: 'local', root: '/p' },
    projectRoots: [{ path: '/p' }, { path: '/q' }],
    additionalRoots: ['/q'],
    expandedDirs: { '/p': true, '/p/src': true, '/p/closed': false } as Record<string, boolean>,
    childrenByDir: { '/p': [], '/p/src': [], '/q': [] } as Record<string, unknown>,
    collabIsGuest: false,
    relisted,
    gates,
    relistDir(dir: string) {
      relisted.push(dir);
      const g = deferred<void>();
      gates.set(dir, g);
      return g.promise;
    },
    ...over,
  };
  return h;
}

function poster() {
  const bodies: { client_id: string; roots: string[]; dirs: string[] }[] = [];
  const acks: ReturnType<typeof deferred<unknown>>[] = [];
  const post = (b: (typeof bodies)[number]) => {
    bodies.push(b);
    const d = deferred<unknown>();
    acks.push(d);
    return d.promise;
  };
  return { bodies, acks, post };
}

// Values from the vm realm: compare their JSON shape.
const plain = (v: unknown) => JSON.parse(JSON.stringify(v));
const tickAll = () => new Promise((r) => setTimeout(r, 0));

test('shown folders: primary root, expanded folders, open attached roots — not collapsed ones', () => {
  assert.deepEqual(plain(shownDirs(host())), ['/p', '/p/src', '/q']);
  // An attached root the person collapsed is not shown.
  assert.deepEqual(plain(shownDirs(host({ expandedDirs: { '/p/src': true, '/q': false } }))), ['/p', '/p/src']);
});

test('the lease is posted when what the tree shows changes, re-posted on the heartbeat, released on leave', async () => {
  const h = host();
  const p = poster();
  const live = new IdeTreeLive(h, p.post, 'c1');
  live.sync();
  live.sync(); // unchanged: no second post
  assert.equal(p.bodies.length, 1);
  assert.deepEqual(plain(p.bodies[0]), { client_id: 'c1', roots: ['/p', '/q'], dirs: ['/p', '/p/src', '/q'] });
  live.sync(true); // heartbeat
  assert.equal(p.bodies.length, 2);
  h.expandedDirs = { ...h.expandedDirs, '/p/closed': true };
  live.sync();
  assert.deepEqual(plain(p.bodies[2].dirs), ['/p', '/p/closed', '/p/src', '/q']);
  // Switching to a Remote SSH target releases the lease once.
  h.target = { kind: 'ssh', root: '/srv' };
  live.sync();
  live.sync();
  assert.equal(p.bodies.length, 4);
  assert.deepEqual(plain(p.bodies[3]), { client_id: 'c1', roots: [], dirs: [] });
  live.release(); // nothing left to release
  assert.equal(p.bodies.length, 4);
});

test('a collab guest never leases', () => {
  const p = poster();
  new IdeTreeLive(host({ collabIsGuest: true }), p.post, 'c1').sync();
  assert.equal(p.bodies.length, 0);
});

test('once the daemon has the lease, newly shown folders are re-read once (no gap before the watcher knew)', async () => {
  const h = host();
  const p = poster();
  const live = new IdeTreeLive(h, p.post, 'c1');
  live.sync();
  p.acks[0].resolve({ watched: ['/p', '/q'] });
  await tickAll();
  assert.deepEqual(h.relisted.sort(), ['/p', '/p/src', '/q']);
  for (const g of h.gates.values()) g.resolve();
  await tickAll();
  h.relisted.length = 0;
  h.expandedDirs = { ...h.expandedDirs, '/p/closed': true, '/p/fresh': true };
  h.childrenByDir = { ...h.childrenByDir, '/p/closed': [] }; // '/p/fresh' is listed fresh when it shows
  live.sync();
  p.acks[1].resolve({ watched: ['/p', '/q'] });
  await tickAll();
  assert.deepEqual(h.relisted, ['/p/closed']);
});

test('a folder that may have changed off screen is re-listed once it comes on screen', async () => {
  const h = host();
  const live = new IdeTreeLive(h, poster().post, 'c1');
  live.onScreen(new Set(['/p']));
  live.apply('c1', ['/p', '/p/src']);
  assert.deepEqual(h.relisted, ['/p']);
  live.onResync(10_000);
  assert.deepEqual(h.relisted, ['/p']); // '/p' still in flight; the rest waits
  live.onScreen(new Set(['/p', '/p/src', '/q']));
  assert.deepEqual(plain(h.relisted.sort()), ['/p', '/p/src', '/q']);
  live.onScreen(new Set(['/p/src']));
  assert.equal(h.relisted.length, 3); // once
});

test('an event re-lists only the named folders still shown, for its own tree only', async () => {
  const h = host();
  const live = new IdeTreeLive(h, poster().post, 'c1');
  live.apply('other-tree', ['/p/src']);
  assert.deepEqual(h.relisted, []);
  live.apply('c1', ['/p/src', '/p/closed', '/elsewhere']);
  assert.deepEqual(h.relisted, ['/p/src']);
});

test('a burst while a listing is in flight re-reads that folder exactly once more', async () => {
  const h = host();
  const live = new IdeTreeLive(h, poster().post, 'c1');
  live.apply('c1', ['/p/src']);
  live.apply('c1', ['/p/src']);
  live.apply('c1', ['/p/src']);
  assert.deepEqual(h.relisted, ['/p/src']);
  h.gates.get('/p/src')!.resolve();
  await tickAll();
  assert.deepEqual(h.relisted, ['/p/src', '/p/src']);
  h.gates.get('/p/src')!.resolve();
  await tickAll();
  assert.equal(h.relisted.length, 2);
});

test('focus re-lists only what nothing watches; a resync always re-lists', async () => {
  const h = host();
  const p = poster();
  const live = new IdeTreeLive(h, p.post, 'c1');
  live.sync();
  p.acks[0].resolve({ watched: ['/p', '/q'] });
  await tickAll();
  for (const g of h.gates.values()) g.resolve();
  await tickAll();
  h.relisted.length = 0;
  live.onFocus(10_000); // watched: only the lease is renewed
  assert.deepEqual(h.relisted, []);
  assert.equal(p.bodies.length, 2);
  live.onResync(20_000);
  assert.deepEqual(plain(h.relisted.sort()), ['/p', '/p/src', '/q']);
});

test('focus re-lists every shown folder of an unwatched (SSH) tree, at most once per 2 s', () => {
  const h = host({ target: { kind: 'ssh', root: '/p' } });
  const live = new IdeTreeLive(h, poster().post, 'c1');
  live.onFocus(10_000);
  assert.deepEqual(h.relisted.sort(), ['/p', '/p/src', '/q']);
  for (const g of h.gates.values()) g.resolve();
  live.onFocus(11_000);
  assert.equal(h.relisted.length, 3);
});
