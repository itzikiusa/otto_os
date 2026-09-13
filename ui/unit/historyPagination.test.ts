import {test} from 'node:test';
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import {runInNewContext} from 'node:vm';
import ts from 'typescript';
function setup(get: (url: string) => Promise<any>) {
  const context = {exports: {} as Record<string, any>, $state: (v: any) => v,
    $derived: Object.assign((v: any) => v, {by: (f: () => any) => f()}), URLSearchParams,
    require: (p: string) => p.endsWith('/client') ? {api: {get}} : {activity: {}}};
  runInNewContext(ts.transpileModule(readFileSync(new URL('../src/modules/agents/history/history.svelte.ts', import.meta.url), 'utf8'), {compilerOptions: {module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022}}).outputText, context);
  return context.exports.history;
}
test('empty candidate page remains actionable and next request uses scanned cursor', async () => {
  const urls: string[] = [];
  const h = setup(async url => {
    urls.push(url);
    if (!url.includes('/history/page?')) return [];
    return urls.length === 1 ? {entries: [], next_cursor: 'scanned-400'} : {entries: [{session_id: 'match', last_active_at: 'same'}], next_cursor: null};
  });
  await h.load('ws'); assert.equal(h.hasMore, true); assert.equal(h.entries.length, 0);
  await h.loadMore(); assert.ok(urls[1].includes('cursor=scanned-400')); assert.equal(h.entries[0].session_id, 'match'); assert.equal(h.hasMore, false);
});
test('a changed filter discards an in-flight continuation and keeps the fresh cursor', async () => {
  let finish!: (v: any) => void; let count = 0;
  const h = setup(async url => {
    count++;
    if (count === 2) return new Promise(resolve => {finish = resolve;});
    const entries = [{session_id: url.includes('q=new') ? 'new' : 'old', last_active_at: 'tie'}];
    return url.includes('/history/page?') ? {entries, next_cursor: 'more'} : entries;
  });
  await h.load('ws'); h.hasMore = true;
  const pending = h.loadMore(); h.q = 'new'; await h.load('ws'); finish({entries: [{session_id: 'stale'}], next_cursor: null}); await pending;
  assert.equal(h.entries[0].session_id, 'new'); assert.equal(h.hasMore, true);
});
