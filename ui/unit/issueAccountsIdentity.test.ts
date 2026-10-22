// ui-correctness-4-03: the shared issue-account cache is per identity — a
// sign-out / impersonation must never offer the previous user's accounts.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { runInNewContext } from 'node:vm';
import ts from 'typescript';
import { strictRequire } from './strictRequire.ts';

function setup(listFor: (who: string | null) => Promise<unknown[]>) {
  const auth = { me: { id: 'admin' } as { id: string } | null };
  let calls = 0;
  const context = {
    exports: {} as Record<string, any>,
    $state: Object.assign((v: unknown) => v, { snapshot: (v: unknown) => v, raw: (v: unknown) => v }),
    Promise,
    require: strictRequire([
      ['../api/issues', { listIssueAccounts: () => { calls++; return listFor(auth.me?.id ?? null); } }],
      ['../loadError', { loadErrorText: (e: unknown) => String(e) }],
      ['./auth.svelte', { auth }],
    ]),
  };
  runInNewContext(
    ts.transpileModule(readFileSync(new URL('../src/lib/stores/issueAccounts.svelte.ts', import.meta.url), 'utf8'), {
      compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 },
    }).outputText,
    context,
  );
  return { store: context.exports.issueAccounts, auth, calls: () => calls };
}

test('a different signed-in identity reloads instead of reusing the cache', async () => {
  const { store, auth, calls } = setup(async (who) => [{ id: `acc-${who}` }]);
  assert.deepEqual((await store.load()).map((a: any) => a.id), ['acc-admin']);
  await store.load();
  assert.equal(calls(), 1, 'same identity: served from cache');
  auth.me = { id: 'u1' };
  assert.deepEqual((await store.load()).map((a: any) => a.id), ['acc-u1']);
  assert.equal(calls(), 2);
  assert.deepEqual(store.list.map((a: any) => a.id), ['acc-u1']);
});

test('a load started by the previous identity never fills the next one’s cache', async () => {
  let release!: () => void;
  const gate = new Promise<void>((r) => { release = r; });
  const { store, auth } = setup(async (who) => { if (who === 'admin') await gate; return [{ id: `acc-${who}` }]; });
  const stale = store.load();
  auth.me = { id: 'u1' };
  const fresh = await store.load();
  release();
  await stale;
  assert.deepEqual(fresh.map((a: any) => a.id), ['acc-u1']);
  assert.deepEqual(store.list.map((a: any) => a.id), ['acc-u1']);
});

test('reset() empties the cache', async () => {
  const { store } = setup(async () => [{ id: 'x' }]);
  await store.load();
  store.reset();
  assert.equal(store.loaded, false);
  assert.equal(store.list.length, 0);
  assert.equal(store.error, null);
});
