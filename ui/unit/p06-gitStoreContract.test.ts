import { test } from 'node:test';
import assert from 'node:assert/strict';
import { loadSource } from './sourceHarness.ts';
import * as livePaths from '../src/lib/gitLivePaths.ts';
import * as latest from '../src/lib/latest.ts';

// architecture-6-04: the git store owns the post-mutation contract. Every push,
// checkout and pull stores the fresh status and bumps `refsRev` (so an open
// graph re-syncs) whoever calls it; `refs()` is the one loader and never throws.

const status = (branch: string, ahead = 0) => ({ branch, upstream: `origin/${branch}`, ahead, behind: 0, changes: [] });

function fixture(fail = false) {
  const calls: { method: string; url: string; body: unknown }[] = [];
  const reply = (url: string) => {
    if (fail) return Promise.reject(new Error('boom'));
    if (url.endsWith('/refs')) return Promise.resolve({ local: [], remote: [], tags: [] });
    if (url.endsWith('/pull')) return Promise.resolve({ status: status('main', 0), note: null });
    return Promise.resolve(status(url.endsWith('/checkout') ? 'feature' : 'main', 0));
  };
  const client = {
    api: {
      get: (url: string) => { calls.push({ method: 'GET', url, body: undefined }); return reply(url); },
      post: (url: string, body?: unknown) => { calls.push({ method: 'POST', url, body }); return reply(url); },
    },
  };
  const imports: Record<string, unknown> = {
    '../api/client': client,
    'lib/api/client': client,
    '../loadError': { loadErrorText: (e: unknown) => String(e) },
    '../gitLivePaths': livePaths,
    '../latest': latest,
  };
  const { git } = loadSource(new URL('../src/lib/stores/git.svelte.ts', import.meta.url), imports, {
    document: { hidden: false, hasFocus: () => true, addEventListener() {}, removeEventListener() {} }, setTimeout, clearTimeout,
  }, { relative: true });
  return { git, calls };
}

test('push stores the status and bumps refsRev (Create PR used to skip both)', async () => {
  const { git, calls } = fixture();
  git.statusById = { r1: status('main', 2) };
  await git.push('r1', { branch: 'feature' });
  assert.deepEqual(calls.map((c) => `${c.method} ${c.url}`), ['POST /repos/r1/push']);
  assert.deepEqual(JSON.parse(JSON.stringify(calls[0].body)), { branch: 'feature' });
  assert.equal(git.statusById.r1.ahead, 0);
  assert.equal(git.refsRev.r1, 1);
});

test('checkout and pull follow the same contract', async () => {
  const { git } = fixture();
  await git.checkout('r1', { branch: 'feature', create: true });
  assert.equal(git.statusById.r1.branch, 'feature');
  assert.equal(git.refsRev.r1, 1);
  const r = await git.pull('r1');
  assert.equal(r.status.branch, 'main');
  assert.equal(git.statusById.r1.branch, 'main');
  assert.equal(git.refsRev.r1, 2);
});

test('a failed mutation leaves status and refsRev alone', async () => {
  const { git } = fixture(true);
  await assert.rejects(git.push('r1'));
  await assert.rejects(git.checkout('r1', { branch: 'x' }));
  assert.equal(git.refsRev.r1, undefined);
  assert.equal(git.statusById.r1, undefined);
});

test('refs() is one loader that reports a failure instead of throwing', async () => {
  const ok = await fixture().git.refs('r1');
  assert.equal(ok.error, null);
  assert.deepEqual(JSON.parse(JSON.stringify(ok.refs)), { local: [], remote: [], tags: [] });
  const bad = await fixture(true).git.refs('r1');
  assert.equal(bad.refs, null);
  assert.match(String(bad.error), /boom/);
});
