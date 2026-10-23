// arch-6-03: the API client store is a façade over lib/stores/apiClient/*
// (tabs, drafts, execute, curl, cookies, collectionsSync, model). These run
// the REAL store and its REAL parts in one sandbox with a fake daemon `api`,
// pinning the behaviour each moved part owns: the tab a close lands on keeps
// its response slot, a failed Postman sync changes nothing, curl import →
// export round-trips, and the cookie list empties only after the daemon did.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { runInNewContext } from 'node:vm';
import { randomUUID } from 'node:crypto';
import ts from 'typescript';
import { apiClientParts } from './apiClientParts.ts';
import { strictRequire } from './strictRequire.ts';
import * as loadError from '../src/lib/loadError.ts';
import { HistoryRefresh, HistoryDetail } from '../src/lib/stores/apiHistory.ts';
import * as scriptRuntime from '../src/lib/api/scripts.ts';
import * as graphqlVars from '../src/lib/api/graphqlVars.ts';
import * as secretShapes from '../src/lib/api/apiSecretShapes.ts';

type Fake = Record<string, (...a: any[]) => Promise<unknown>>;

function setup(api: Fake) {
  const toasts: { kind: string; title: string }[] = [];
  const fake: Record<string, any> = { get: async () => [], post: async () => ({}), patch: async () => ({}), del: async () => undefined, ...api };
  fake.long = { post: (...a: unknown[]) => fake.post(...a), get: (...a: unknown[]) => fake.get(...a) };
  const parts = apiClientParts();
  const context = {
    exports: {} as Record<string, any>,
    $state: Object.assign((v: unknown) => v, { snapshot: (v: unknown) => v, raw: (v: unknown) => v }), $derived: (v: unknown) => v,
    crypto: { randomUUID }, URL, URLSearchParams, AbortController, DOMException, setTimeout, clearTimeout, performance, encodeURIComponent,
    localStorage: { getItem() { return null; }, setItem() {} },
    require: strictRequire([
      ...parts.entries,
      ['/announce.svelte', { announce() {} }],
      ['/api/client', { api: fake, isAbortError: (e: any) => e?.name === 'AbortError', newHostConfirmHost: () => null }],
      ['/workspace.svelte', { ws: { currentId: 'W', myRole: 'owner' } }],
      ['/toast.svelte', { toasts: {
        error: (title: string) => { toasts.push({ kind: 'error', title }); },
        success: (title: string) => { toasts.push({ kind: 'success', title }); },
        info: (title: string) => { toasts.push({ kind: 'info', title }); },
      } }],
      ['/confirm.svelte', { confirmer: new Proxy({}, { get: (_t, k) => () => { throw new Error(`unexpected confirmer.${String(k)}`); } }) }],
      ['/apiHistory', { HistoryRefresh, HistoryDetail }],
      ['/apiSecretShapes', secretShapes],
      ['/scriptRunner', { runScript: async () => { throw new Error('no scripts in these flows'); } }],
      ['/graphqlVars', graphqlVars],
      ['/lazyModule', { announceModule() {} }],
      ['/plural', { plural: (n: number, w: string) => `${n} ${w}${n === 1 ? '' : 's'}` }],
      ['/api/scripts', scriptRuntime],
      ['/importers', { isImportedEnvironment: () => false, detectAndParse: () => { throw new Error('not reached'); } }],
      ['/api/types', { isSecretRef: (v: any) => !!v?.$secret }],
      ['/loadError', loadError],
    ]),
  };
  parts.load(context);
  runInNewContext(ts.transpileModule(readFileSync(new URL('../src/lib/stores/apiClient.svelte.ts', import.meta.url), 'utf8'),
    { compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 } }).outputText, context);
  return { v: context.exports.apiClient, toasts };
}

const ok = (status: number, body: string) => ({ status, status_text: 'OK', headers: [], body, duration_ms: 1 });

test('closing the active tab lands on its neighbour, which keeps its own response slot', async () => {
  const { v } = setup({ post: async (_url: string, body: any) => ok(200, `sent:${body.url}`) });
  v.draft = { ...v.draft, url: 'https://a.test' };
  v.newDraft(); v.draft = { ...v.draft, url: 'https://b.test' };
  assert.ok(await v.execute());
  v.newDraft(); v.draft = { ...v.draft, url: 'https://c.test' };
  assert.ok(await v.execute());
  const [, b, c] = v.tabs.map((t: any) => t.tabId);

  // Closing the LAST tab while it is active: the left neighbour takes over
  // and shows its own response (not C's, not a blank pane).
  v.closeTab(2);
  assert.equal(v.activeTab, 1);
  assert.equal(v.draft.tabId, b);
  assert.equal(v.lastResponse?.body, 'sent:https://b.test');
  assert.equal(v.responses.has(c), false, 'the closed tab’s slot is dropped');

  // Closing an active tab in the MIDDLE: the right neighbour slides into
  // its index and keeps its slot.
  v.newDraft(); v.draft = { ...v.draft, url: 'https://d.test' };
  assert.ok(await v.execute());
  const d = v.draft.tabId;
  v.switchTab(1);
  v.closeTab(1);
  assert.equal(v.draft.tabId, d);
  assert.equal(v.lastResponse?.body, 'sent:https://d.test');
  assert.equal(v.tabStatus(d), 'ok');
});

test('a failed Postman sync returns false, toasts, and leaves the collections as they were', async () => {
  const collections = [{ id: 'c1', name: 'Kept', parent_id: null }];
  const posts: string[] = [];
  const { v, toasts } = setup({
    get: async (url: string) => (url.endsWith('/collections') ? collections : []),
    post: async (url: string) => { posts.push(url); throw Object.assign(new Error('401 bad key'), { status: 401 }); },
  });
  await v.loadAll();
  assert.deepEqual(v.collections.map((c: any) => c.name), ['Kept']);

  assert.equal(await v.postmanSync('bad-key', false), false);
  assert.deepEqual(posts, ['/workspaces/W/api-client/postman/sync'], 'nothing is imported after the failed sync call');
  assert.deepEqual(v.collections.map((c: any) => c.name), ['Kept']);
  assert.deepEqual(toasts.map((t) => `${t.kind}:${t.title}`), ['error:Couldn’t sync with Postman']);
});

test('importCurl → toCurl round-trips the method, headers and body', async () => {
  const curl = `curl -X POST 'https://h.test/p' -H 'X-Trace: 1' -H 'Content-Type: application/json' --data '{"a":1}'`;
  let sent: unknown;
  const { v } = setup({
    post: async (url: string, body: any) => {
      assert.equal(url, '/api-client/import-curl');
      sent = body;
      return {
        method: 'POST', url: 'https://h.test/p', query: [], body_mode: 'json', body: '{"a":1}', auth: { type: 'none' },
        headers: [{ key: 'X-Trace', value: '1', enabled: true }, { key: 'Content-Type', value: 'application/json', enabled: true }],
      };
    },
  });
  assert.equal(await v.importCurl(curl), true);
  assert.equal((sent as { curl: string }).curl, curl);
  assert.equal(v.draft.method, 'POST');
  assert.equal(v.draft.body, '{"a":1}');
  assert.equal(v.toCurl(), curl);
  assert.equal(v.toCurl(v.draft), curl);
});

test('clearing cookies empties the list only after the daemon cleared the jar', async () => {
  const jar = [{ name: 'sid', value: '1', domain: 'h.test', path: '/' }, { name: 'csrf', value: '2', domain: 'h.test', path: '/' }];
  let finish!: (fail?: boolean) => void;
  const { v, toasts } = setup({
    get: async (url: string) => (url.endsWith('/cookies') ? jar : []),
    del: (url: string) => {
      assert.equal(url, '/workspaces/W/api-client/cookies');
      return new Promise((resolve, reject) => { finish = (fail) => (fail ? reject(new Error('daemon down')) : resolve(undefined)); });
    },
  });
  await v.loadCookies();
  assert.equal(v.cookies.length, 2);

  // A failed clear keeps the rows: they are still in the jar.
  let clearing = v.clearCookies();
  assert.equal(v.cookies.length, 2, 'nothing is removed while the call is in flight');
  finish(true); await clearing;
  assert.equal(v.cookies.length, 2);
  assert.deepEqual(toasts.map((t) => `${t.kind}:${t.title}`), ['error:Couldn’t clear cookies']);

  clearing = v.clearCookies();
  assert.equal(v.cookies.length, 2);
  finish(); await clearing;
  assert.equal(v.cookies.length, 0);
  assert.equal(toasts.at(-1)?.title, 'Cookies cleared');
});
