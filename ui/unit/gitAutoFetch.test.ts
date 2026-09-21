import { test } from 'node:test';
import assert from 'node:assert/strict';
import { deferred, loadSource } from './sourceHarness.ts';

const status = { branch: 'main', upstream: 'origin/main', ahead: 0, behind: 0, changes: [] };
const flush = async () => { for (let i = 0; i < 20; i++) await Promise.resolve(); };
function fixture() {
  let now = 1_000_000;
  let focused = true;
  const document = { hidden: false, hasFocus: () => focused };
  const timers = new Map<number, { at: number; run: () => void }>();
  let timerId = 0;
  const calls: { id: string; result: ReturnType<typeof deferred<typeof status>> }[] = [];
  const { git } = loadSource(new URL('../src/lib/stores/git.svelte.ts', import.meta.url), {
    '../api/client': { api: { post: (url: string) => {
      const result = deferred<typeof status>();
      calls.push({ id: url.split('/')[2], result });
      return result.promise;
    } } },
  }, {
    document, Date: { now: () => now },
    setTimeout: (fn: () => void, ms: number) => { timers.set(++timerId, { at: now + ms, run: fn }); return timerId; },
    clearTimeout: (id: number) => timers.delete(id),
  });
  return { git, calls, document, timers,
    focus: (value: boolean) => { focused = value; },
    advance: async (ms: number) => {
      now += ms;
      const pending = [...timers].filter(([, timer]) => timer.at <= now);
      pending.forEach(([id, timer]) => { timers.delete(id); timer.run(); }); await flush();
    },
  };
}

test('open tabs fetch immediately, selected first, with at most two requests', async () => {
  const f = fixture();
  f.git.openRepoIds = ['one', 'two', 'three']; f.git.activeRepoId = 'three';
  f.git.startAutoFetch(); await f.advance(0);
  assert.deepEqual(f.calls.map((c) => c.id), ['three', 'one']);
  f.calls[0].result.resolve(status); await flush();
  assert.deepEqual(f.calls.map((c) => c.id), ['three', 'one', 'two']);
  f.calls[1].result.resolve(status); f.calls[2].result.resolve(status); await flush();
  assert.equal(f.git.refsRev.three, 1);
  await f.advance(30_000);
  assert.deepEqual(f.calls.map((c) => c.id), ['three', 'one', 'two', 'three']);
  f.calls[3].result.resolve(status); await flush();
  await f.advance(90_000);
  assert.equal(f.calls.length, 6);
  f.git.stopAutoFetch();
});

test('manual fetch joins background request and always invalidates unchanged refs', async () => {
  const f = fixture(); f.git.openRepoIds = ['one']; f.git.activeRepoId = 'one';
  f.git.setStatus('one', status);
  f.git.startAutoFetch(); await f.advance(0);
  const manual = f.git.fetchRepo('one');
  assert.equal(f.calls.length, 1);
  f.calls[0].result.resolve(status); await manual; await flush();
  assert.equal(f.git.refsRev.one, 1);
  f.git.requestAutoFetch(); await f.advance(0);
  assert.equal(f.calls.length, 1);
  f.git.stopAutoFetch();
});

test('hidden or unfocused windows defer fetches; closed tabs never start queued work', async () => {
  const f = fixture(); f.git.openRepoIds = ['one', 'two', 'three'];
  f.document.hidden = true; f.git.startAutoFetch(); await f.advance(0);
  assert.equal(f.calls.length, 0);
  f.document.hidden = false; f.focus(false); f.git.requestAutoFetch(); await f.advance(0);
  assert.equal(f.calls.length, 0);
  f.focus(true); f.git.requestAutoFetch(); await f.advance(0);
  assert.equal(f.calls.length, 2);
  f.git.closeRepoTab('three'); f.calls[0].result.resolve(status); f.calls[1].result.resolve(status); await flush();
  assert.equal(f.calls.length, 2);
  f.git.stopAutoFetch();
});

test('failure backs off despite focus events; manual retry surfaces errors and clears backoff on success', async () => {
  const f = fixture(); f.git.openRepoIds = ['one']; f.git.activeRepoId = 'one';
  f.git.startAutoFetch(); await f.advance(0);
  f.calls[0].result.reject(new Error('offline')); await flush();
  await f.advance(30_000); f.git.requestAutoFetch(); await f.advance(0);
  assert.equal(f.calls.length, 1);
  const failure = f.git.fetchRepo('one');
  f.calls[1].result.reject(new Error('auth failed'));
  await assert.rejects(failure, /auth failed/); await flush();
  const manual = f.git.fetchRepo('one'); f.calls[2].result.resolve(status); await manual; await flush();
  await f.advance(30_000);
  assert.equal(f.calls.length, 4);
  f.git.setAutoFetch(false); f.calls[3].result.resolve(status); await flush();
  await f.advance(600_000); assert.equal(f.calls.length, 4);
  f.git.stopAutoFetch();
});


test('restart during an in-flight round does not overlap or revive the stopped timer', async () => {
  const f = fixture(); f.git.openRepoIds = ['one']; f.git.activeRepoId = 'one';
  f.git.startAutoFetch(); await f.advance(0);
  f.git.stopAutoFetch(); f.git.startAutoFetch(); await f.advance(0);
  assert.equal(f.calls.length, 1);
  f.calls[0].result.resolve(status); await flush();
  assert.equal(f.timers.size, 1);
  await f.advance(30_000);
  assert.equal(f.calls.length, 2);
  f.git.stopAutoFetch(); f.calls[1].result.resolve(status); await flush();
  assert.equal(f.timers.size, 0);
});

test('manual requests share the global two-request limit', async () => {
  const f = fixture();
  const first = f.git.fetchRepo('one');
  const second = f.git.fetchRepo('two');
  const third = f.git.fetchRepo('three');
  assert.equal(f.calls.length, 2);
  f.calls[0].result.resolve(status); await first; await flush();
  assert.equal(f.calls.length, 3);
  f.calls[1].result.resolve(status); f.calls[2].result.resolve(status);
  await Promise.all([second, third]);
});

test('a failed tab bootstrap can retry when the Git page opens later', async () => {
  const f = fixture(); let loads = 0; let restores = 0;
  f.git.loadAllRepos = async () => { f.git.allReposLoaded = ++loads > 1; };
  f.git.restoreOpenTabs = () => { restores++; };
  await f.git.initializeOpenTabs();
  await f.git.initializeOpenTabs();
  assert.equal(loads, 2);
  assert.equal(restores, 1);
  await f.git.initializeOpenTabs();
  assert.equal(loads, 2);
});

test('background fetch queued behind manual work rechecks close, stop, hidden and pause before starting', async () => {
  for (const cancel of ['close', 'stop', 'hide', 'pause']) {
    const f = fixture();
    const first = f.git.fetchRepo('manual-one'); const second = f.git.fetchRepo('manual-two');
    f.git.openRepoIds = ['queued']; f.git.startAutoFetch(); await f.advance(0);
    if (cancel === 'close') f.git.closeRepoTab('queued');
    if (cancel === 'stop') f.git.stopAutoFetch();
    if (cancel === 'hide') f.document.hidden = true;
    if (cancel === 'pause') f.git.setAutoFetch(false);
    f.calls[0].result.resolve(status); await first; await flush();
    assert.equal(f.calls.length, 2, cancel);
    f.calls[1].result.resolve(status); await second; f.git.stopAutoFetch();
  }
});

test('manual caller promotes a queued background fetch and retains its request after tab closure', async () => {
  const f = fixture();
  const first = f.git.fetchRepo('manual-one'); const second = f.git.fetchRepo('manual-two');
  f.git.openRepoIds = ['queued']; f.git.startAutoFetch(); await f.advance(0);
  const explicit = f.git.fetchRepo('queued');
  f.git.closeRepoTab('queued'); f.git.stopAutoFetch();
  f.calls[0].result.resolve(status); await first; await flush();
  assert.deepEqual(f.calls.map((c) => c.id), ['manual-one', 'manual-two', 'queued']);
  f.calls[1].result.resolve(status); f.calls[2].result.resolve(status);
  await Promise.all([second, explicit]);
});
