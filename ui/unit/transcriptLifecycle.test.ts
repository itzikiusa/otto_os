import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { runInNewContext } from 'node:vm';
import ts from 'typescript';
const page = () => ({turns: [{id: 'turn', blocks: [], role: 'user'}], stats: {turns: 1}, cursor: '0', has_earlier: false, provider: 'claude'});
const settle = async () => {for (let i = 0; i < 16; i++) await Promise.resolve();};
function setup(get?: (url: string, signal?: AbortSignal) => Promise<any>) {
  const gets: string[] = [], posts: string[] = [], storage = new Map<string, string>();
  const browserStorage = {getItem: (k: string) => storage.get(k) ?? null, setItem: (k: string, v: string) => storage.set(k, v), removeItem: (k: string) => storage.delete(k)};
  const document = {hidden: false};
  const globals = {$state: (v: unknown) => v, document, AbortController, URLSearchParams, setTimeout, clearTimeout, localStorage: browserStorage, sessionStorage: browserStorage};
  const load = (file: URL): Record<string, any> => {
    const context = {...globals, exports: {} as Record<string, any>, require: (p: string): any => {
      if (p.endsWith('/client')) return {api: {get: async (url: string, signal?: AbortSignal) => {gets.push(url); return get ? get(url, signal) : page();}, post: async (url: string) => {posts.push(url);}}};
      if (p.endsWith('/win')) return {winKey: (k: string) => k};
      if (p.endsWith('transcriptLifecycle')) return load(new URL('../src/lib/stores/transcriptLifecycle.ts', import.meta.url));
      return {};
    }};
    runInNewContext(ts.transpileModule(readFileSync(file, 'utf8'), {compilerOptions: {module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022}}).outputText, context);
    return context.exports;
  };
  const store = load(new URL('../src/lib/stores/transcript.svelte.ts', import.meta.url)).transcript;
  // Before leases exist, exercise the actual previous mounted-load behavior.
  const acquire = (id: string) => store.acquireView ? store.acquireView({sessionId: id}) : (store.ensure({sessionId: id}), () => {});
  const resync = () => store.resyncVisible ? store.resyncVisible() : store.resyncAll('ws');
  return {store, gets, posts, document, acquire, resync, storage};
}
test('reconnect resyncs leased views only, never a cached closed session or workspace', async () => {
  const h = setup(), closeA = h.acquire('a'), closeB = h.acquire('b'); await settle(); closeB(); h.gets.length = 0; h.posts.length = 0;
  h.resync(); await settle(); assert.deepEqual(h.gets, ['/sessions/a/transcript?limit=60']); assert.deepEqual(h.posts, ['/sessions/a/transcript/touch']); closeA();
});
test('closing a pending GET prevents late touch and state publication', async () => {
  let finish!: (v: any) => void; const h = setup(() => new Promise(resolve => {finish = resolve;}));
  const close = h.acquire('a'); h.resync(); await settle(); close(); finish(page()); await settle();
  assert.equal(h.posts.length, 0); assert.equal(h.store.peek('a').transcript, null);
});
test('two panes share one load and releasing one leaves the other live', async () => {
  const h = setup(), one = h.acquire('a'), two = h.acquire('a'); await settle(); assert.equal(h.gets.length, 1);
  one(); h.gets.length = 0; h.resync(); await settle(); assert.equal(h.gets.length, 1);
  two(); h.gets.length = 0; h.resync(); await settle(); assert.equal(h.gets.length, 0);
});
test('hidden document never resyncs or resumes a leased session', async () => {
  const h = setup(), close = h.acquire('a'); await settle(); h.document.hidden = true; h.store.setVisible?.(false); h.gets.length = 0; h.posts.length = 0;
  h.resync(); await settle(); assert.equal(h.gets.length, 0); assert.equal(h.posts.length, 0); close();
});
test('resync storms bound reads to two and one trailing request per source', async () => {
  const waiting: (() => void)[] = []; let active = 0, peak = 0;
  const h = setup(async () => {active++; peak = Math.max(peak, active); await new Promise<void>(resolve => waiting.push(resolve)); active--; return page();});
  const closes = ['a', 'b', 'c', 'd'].map(h.acquire); await settle(); for (let i = 0; i < 20; i++) h.resync(); assert.equal(peak, 2);
  for (let i = 0; i < 10; i++) {waiting.splice(0).forEach(f => f()); await settle();} assert.ok(h.gets.length <= 8); closes.forEach(f => f());
});
test('inactive cache is bounded without deleting drafts or active older pages', async () => {
  const h = setup(), active = h.acquire('active'); await settle(); h.store.peek('active').turns = Array.from({length: 120}, (_, i) => ({id: String(i), blocks: []})); h.store.setDraft('s0', 'unsent');
  for (let i = 0; i < 30; i++) {const close = h.acquire(`s${i}`); await settle(); close();}
  assert.equal(h.store.peek('s0'), null); assert.equal(h.store.draft('s0'), 'unsent'); assert.equal(h.store.peek('active').turns.length, 120); active();
});
test('separate windows never release each other leases', async () => {
  const a = setup(), b = setup(), closeA = a.acquire('a'), closeB = b.acquire('a'); await settle(); closeA(); b.gets.length = 0; b.resync(); await settle(); assert.equal(b.gets.length, 1); closeB();
});
test('hidden pane release does not evict or abort its sibling lease on return', async () => {
  const h = setup(), first = h.acquire('a'), second = h.acquire('a'); await settle();
  h.document.hidden = true; h.store.setVisible(false); first();
  for (let i = 0; i < 30; i++) { const close = h.acquire(`other${i}`); close(); }
  assert.notEqual(h.store.peek('a'), null);
  h.document.hidden = false; h.store.setVisible(true); await settle();
  assert.equal(h.store.peek('a').turns.length, 1); second();
});
test('identity reset revokes in-flight reads and clears bodies without deleting a draft', async () => {
  let finish!: (v:any)=>void; const h=setup(()=>new Promise(resolve=>{finish=resolve;}));
  h.store.setIdentity?.('daemon/user-a'); h.store.setDraft('a','unsent'); const close=h.acquire('a'); await settle();
  h.store.setIdentity?.('daemon/user-b'); finish(page()); await settle();
  assert.equal(h.store.peek('a'),null); assert.equal(h.posts.length,0); assert.equal(h.store.draft('a'),''); h.store.setIdentity('daemon/user-a'); assert.equal(h.store.draft('a'),'unsent'); close();
});

test('existing unscoped draft is migrated once and survives the new storage namespace',()=>{
  const h=setup(); h.storage.set('otto_chat_draft:a','legacy unsent'); h.store.setIdentity('first-owner');
  assert.equal(h.store.draft('a'),'legacy unsent'); h.store.setIdentity('second-owner'); assert.equal(h.store.draft('a'),'');
  h.store.setIdentity('first-owner'); assert.equal(h.store.draft('a'),'legacy unsent');
});
test('busy folds retry only while visible and never touch after the failed read',async(t)=>{
  t.mock.timers.enable({apis:['setTimeout']}); let reads=0;
  const h=setup(async()=>{if(++reads===1) throw new Error('transcript busy; retry shortly');return page();});
  const close=h.acquire('a'); await settle(); assert.equal(h.posts.length,0);
  t.mock.timers.tick(1000); await settle(); assert.equal(reads,2); assert.equal(h.posts.length,1); close();
});
test('closing a subagent read releases loading and ignores a late failure after reopening',async()=>{
  let reject!: (e:Error)=>void; const h=setup(url=>url.includes('sub=')?new Promise((_,r)=>{reject=r;}):Promise.resolve(page()));
  let close=h.acquire('a');await settle();const c=h.store.peek('a');const pending=c.loadSubagent('child');await settle();close();
  assert.equal(c.subagents.child.loading,false);close=h.acquire('a');await settle();reject(new Error('obsolete child failure'));await pending;
  assert.equal(c.subagents.child.error,null);close();
});
test('releasing a large reader does not traverse or serialize its retained body',async()=>{
  const h=setup(),close=h.acquire('a');await settle();const c=h.store.peek('a');
  Object.defineProperty(c.turns[0],'blocks',{get(){throw new Error('release traversed retained turns');}});
  assert.doesNotThrow(close);
});
test('parent Reload preserves accounting for retained child bodies',async()=>{
  const h=setup(url=>Promise.resolve(url.includes('sub=')?{...page(),turns:[{id:'child-turn',role:'assistant',blocks:[{kind:'text',md:'x'.repeat(100_000)}]}]}:page()));
  const close=h.acquire('a');await settle();const c=h.store.peek('a');await c.loadSubagent('child');const before=c.retainedBytes;
  assert.ok(before>=200_000);await c.load();assert.ok(c.retainedBytes>=before,'Reload retains subagents, so their charge must remain');close();
});
