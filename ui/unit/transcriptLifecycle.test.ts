import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { runInNewContext } from 'node:vm';
import ts from 'typescript';
import { strictRequire, unused } from './strictRequire.ts';
const page = () => ({turns: [{id: 'turn', blocks: [], role: 'user'}], stats: {turns: 1}, cursor: '0', has_earlier: false, provider: 'claude'});
const settle = async () => {for (let i = 0; i < 16; i++) await Promise.resolve();};
function setup(get?: (url: string, signal?: AbortSignal) => Promise<any>) {
  const gets: string[] = [], posts: string[] = [], storage = new Map<string, string>();
  const browserStorage = {getItem: (k: string) => storage.get(k) ?? null, setItem: (k: string, v: string) => storage.set(k, v), removeItem: (k: string) => storage.delete(k)};
  const document = {hidden: false};
  const globals = {$state: Object.assign((v: unknown) => v, {raw: (v: unknown) => v}), document, AbortController, URLSearchParams, setTimeout, clearTimeout, localStorage: browserStorage, sessionStorage: browserStorage};
  const load = (file: URL): Record<string, any> => {
    const context = {...globals, exports: {} as Record<string, any>, require: (p: string): any => {
      if (p.endsWith('/client')) { const post = async (url: string) => {posts.push(url);}; return {api: {get: async (url: string, signal?: AbortSignal) => {gets.push(url); return get ? get(url, signal) : page();}, post, bg: {post}}}; }
      if (p.endsWith('/win')) return {winKey: (k: string) => k};
      if (p.endsWith('transcriptLifecycle')) return load(new URL('../src/lib/stores/transcriptLifecycle.ts', import.meta.url));
      return strictRequire([['/paneHeader', unused('/paneHeader')]])(p);
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
  let close=h.acquire('a');await settle();const c=h.store.peek('a');const child=c.acquireSubagent('child');const pending=c.loadSubagent('child');await settle();close();
  assert.equal(c.subagents.child?.loading ?? false,false);child();close=h.acquire('a');await settle();reject(new Error('obsolete child failure'));await pending;
  assert.equal(c.subagents.child?.error ?? null,null);close();
});
test('releasing a large reader does not traverse or serialize its retained body',async()=>{
  const h=setup(),close=h.acquire('a');await settle();const c=h.store.peek('a');
  Object.defineProperty(c.turns[0],'blocks',{get(){throw new Error('release traversed retained turns');}});
  assert.doesNotThrow(close);
});
test('parent Reload preserves accounting for retained child bodies',async()=>{
  const h=setup(url=>Promise.resolve(url.includes('sub=')?{...page(),turns:[{id:'child-turn',role:'assistant',blocks:[{kind:'text',md:'x'.repeat(100_000)}]}]}:page()));
  const close=h.acquire('a');await settle();const c=h.store.peek('a');const child=c.acquireSubagent('child');await c.loadSubagent('child');const before=c.retainedBytes;
  assert.ok(before>=200_000);await c.load();assert.ok(c.retainedBytes>=before,'Reload retains subagents, so their charge must remain');child();close();
});
test('subagent bodies are replaced whole (raw state), never mutated in place',async()=>{
  const h=setup(url=>Promise.resolve(url.includes('sub=')?{...page(),turns:[{id:url.includes('sub=one')?'one-turn':'two-turn',role:'assistant',blocks:[]}]}:page()));
  const close=h.acquire('a');await settle();const c=h.store.peek('a');
  const oneLease=c.acquireSubagent('one'),twoLease=c.acquireSubagent('two');
  await c.loadSubagent('one');const afterOne=c.subagents;const one=afterOne.one;
  await c.loadSubagent('two');
  assert.notEqual(c.subagents,afterOne,'a change swaps the record, so raw state re-renders');
  assert.equal(c.subagents.one,one,'an untouched body keeps its identity');
  assert.equal(afterOne.two,undefined,'the previous record was not mutated');
  assert.deepEqual([c.subagents.one.turns[0].id,c.subagents.two.turns[0].id],['one-turn','two-turn']);
  oneLease();twoLease();close();
});

for (const hasEarlier of [false, true]) test(`disjoint reconnect retains a reachable earlier cursor (old earlier=${hasEarlier})`, async () => {
  const turns = (first: number, last: number) => Array.from({length: last - first + 1}, (_, i) => ({id: String(first + i), blocks: [], role: 'user'}));
  let fresh = false;
  const h = setup(async url => url.includes('before=71')
    ? {...page(), turns: turns(11, 70), cursor: '11', has_earlier: true}
    : {...page(), turns: fresh ? turns(71, 130) : turns(1, 60), cursor: fresh ? '71' : '1', has_earlier: fresh || hasEarlier});
  const close = h.acquire('a'); await settle(); fresh = true; h.resync(); await settle();
  const c = h.store.peek('a');
  assert.equal(c.transcript.has_earlier, true);
  assert.equal(c.transcript.cursor, '71');
  await c.loadEarlier();
  for (let i = 61; i <= 70; i++) assert.ok(c.turns.some((t: any) => t.id === String(i)), `missing turn ${i}`);
  close();
});

test('returning to live following releases paged history and keeps its recovery cursor', async () => {
  const h = setup(async url => ({...page(), has_earlier: true, cursor: url.includes('before=') ? '0' : '60', turns: [{id: url.includes('before=') ? 'old' : 'tail', blocks: [], role: 'user'}]}));
  const close = h.acquire('a'); await settle(); const c = h.store.peek('a');
  await c.loadEarlier();
  for (let i = 1; i <= 650; i++) c.applyDelta(String(60 + i), [{id: `new${i}`, blocks: [], role: 'user'}]);
  assert.ok(c.turns.some((t: any) => t.id === 'old'), 'historical reader keeps the anchor');
  c.followLive();
  assert.ok(c.turns.length <= 600);
  assert.equal(c.transcript.has_earlier, true);
  assert.ok(Number(c.transcript.cursor) > 0);
  for (let i = 651; i <= 1300; i++) c.applyDelta(String(60 + i), [{id: `new${i}`, blocks: [], role: 'user'}]);
  assert.ok(c.turns.length <= 600, 'future deltas remain bounded'); close();
});

test('live following bounds payload bytes as well as turn count', async () => {
  const h = setup(); const close = h.acquire('a'); await settle(); const c = h.store.peek('a');
  // Each frame fits the existing 64 KiB transport guard, but the retained
  // window would otherwise reach ~25 MiB well before its 600-turn limit.
  for (let i = 0; i < 500; i++) c.applyDelta(String(i + 1), [{id: `heavy${i}`, role: 'user', blocks: [{kind: 'text', md: 'x'.repeat(50000)}]}]);
  assert.ok(JSON.stringify(c.turns).length <= 8 * 1024 * 1024);
  assert.equal(c.transcript.has_earlier, true);
  close();
});

// Until explicit child consumers exist, exercise the current card behavior:
// expand loads once and collapse has no release action. The assertions below
// expose retained payload, rather than failing because a new method is absent.
function childLease(c: any, id: string): () => void {
  return c.acquireSubagent ? c.acquireSubagent(id) : () => {};
}
test('collapsing 100 inspected subagents releases bodies while their parent stays mounted', async () => {
  const h = setup(url => Promise.resolve(url.includes('sub=') ? {...page(), turns: [{id: 'child', role: 'assistant', blocks: [{kind: 'text', md: 'x'.repeat(250_000)}]}]} : page()));
  const parent = h.acquire('a'); await settle(); const c = h.store.peek('a');
  for (let i = 0; i < 100; i++) {
    const close = childLease(c, `child${i}`); await c.loadSubagent(`child${i}`); close();
  }
  const bodies = Object.values(c.subagents) as any[];
  assert.ok(bodies.filter(b => b.turns.length > 0).length <= 8, 'released children must not retain 100 body pages');
  assert.ok(JSON.stringify(c.subagents).length <= 8 * 1024 * 1024, 'parent stays mounted under a child-body byte budget');
  const reopen = childLease(c, 'child0'); await c.loadSubagent('child0');
  assert.equal(c.subagents.child0.turns[0].id, 'child', 'an evicted body remains reachable by reopening');
  reopen(); parent();
});
test('paging a 1200-turn child keeps its retained/render input window bounded', async () => {
  const h = setup(async url => {
    if (!url.includes('sub=')) return page();
    const before = new URL(url, 'http://fixture').searchParams.get('before');
    const end = before === null ? 1200 : Number(before), start = Math.max(0, end - 60);
    return {...page(), cursor: String(start), has_earlier: start > 0,
      turns: Array.from({length: end - start}, (_, i) => ({id: String(start + i), role: 'assistant', blocks: [{kind: 'text', md: `child turn ${start + i}`}]}))};
  });
  const parent = h.acquire('a'); await settle(); const c = h.store.peek('a'), close = childLease(c, 'long');
  await c.loadSubagent('long');
  for (let i = 0; i < 19; i++) await c.loadSubagentEarlier('long');
  assert.equal(c.subagents.long.turns[0].id, '0', 'oldest page remains reachable');
  assert.ok(c.subagents.long.turns.length <= 300, 'group/render input may not grow with all 1200 turns');
  assert.equal(typeof c.loadSubagentLater, 'function', 'bounded history must provide a newer-page path');
  for (let i = 0; i < 19; i++) await c.loadSubagentLater('long');
  assert.equal(c.subagents.long.turns.at(-1).id, '1199', 'newer pages remain reachable after bounded eviction');
  close(); parent();
});
test('collapse revokes a pending child read without affecting its mounted parent', async () => {
  let finish!: (v: any) => void;
  const h = setup(url => url.includes('sub=') ? new Promise(resolve => {finish = resolve;}) : Promise.resolve(page()));
  const parent = h.acquire('a'); await settle(); const c = h.store.peek('a'), close = childLease(c, 'pending');
  const read = c.loadSubagent('pending'); await settle(); close(); finish(page()); await read;
  assert.equal(c.subagents.pending?.turns.length ?? 0, 0, 'late body must not repopulate a collapsed card');
  assert.equal(c.turns[0].id, 'turn'); parent();
});

test('active child-body count is shared across parents and admission recovers after collapse', async () => {
  const h = setup(), parentA = h.acquire('a'), parentB = h.acquire('b'); await settle();
  const a = h.store.peek('a'), b = h.store.peek('b'), releases: (() => void)[] = [];
  for (let i = 0; i < 8; i++) {releases.push(a.acquireSubagent(String(i))); await a.loadSubagent(String(i));}
  const releaseB = b.acquireSubagent('ninth'); await b.loadSubagent('ninth');
  assert.equal(b.subagents.ninth.turns.length, 0);
  assert.match(b.subagents.ninth.error, /Close another expanded subagent/);
  releases.shift()!(); await b.loadSubagent('ninth');
  assert.equal(b.subagents.ninth.turns[0].id, 'turn');
  releases.forEach(release => release()); releaseB(); parentA(); parentB();
});
test('two child consumers share a body and only final collapse releases it', async () => {
  const h = setup(), parent = h.acquire('a'); await settle(); const c = h.store.peek('a');
  const one = c.acquireSubagent('shared'), two = c.acquireSubagent('shared'); await c.loadSubagent('shared');
  one(); assert.equal(c.subagents.shared.turns[0].id, 'turn');
  two(); assert.equal(c.subagents.shared, undefined); parent();
});
test('a permanently oversized child turn gives no impossible retry', async () => {
  const h = setup(async url => url.includes('sub=') ? {...page(), turns: [{id: 'large', role: 'assistant', blocks: [{kind: 'text', md: 'x'.repeat(3 * 1024 * 1024)}]}]} : page());
  const parent = h.acquire('a'); await settle(); const c = h.store.peek('a'), close = c.acquireSubagent('large');
  await c.loadSubagent('large'); assert.equal(c.subagents.large.turns.length, 0);
  assert.equal(c.subagents.large.retryable, false); assert.match(c.subagents.large.error, /provider transcript/);
  close(); parent();
});

test('concurrent child expansion reserves capacity before fetching response bodies', async () => {
  const requests: {url: string; signal?: AbortSignal; finish: (value: any) => void}[] = [];
  const h = setup((url, signal) => url.includes('sub=') ? new Promise(resolve => requests.push({url, signal, finish: resolve})) : Promise.resolve(page()));
  const parent = h.acquire('a'); await settle(); const c = h.store.peek('a');
  const releases = Array.from({length: 100}, (_, i) => c.acquireSubagent(`concurrent${i}`));
  const loads = Array.from({length: 100}, (_, i) => c.loadSubagent(`concurrent${i}`));
  await settle();
  try {
    assert.ok(requests.length <= 8, `100 expansions launched ${requests.length} child fetches before admission`);
    const initial = requests.length;
    releases[0]();
    assert.equal(requests[0].signal?.aborted, true);
    const retry = c.loadSubagent('concurrent99'); await settle();
    assert.equal(requests.length, initial + 1, 'collapse releases a reserved slot for a refused visible card');
    requests.forEach(request => request.finish(page())); await retry; await Promise.all(loads);
    assert.equal(c.subagents.concurrent99.turns[0].id, 'turn');
  } finally {
    releases.forEach(release => release()); parent(); requests.forEach(request => request.finish(page()));
    await Promise.all(loads);
  }
});

test('shared child byte budget credits replacement pages and preserves a refused destination for retry', async () => {
  const h = setup(async url => {
    const q = new URL(url, 'http://fixture').searchParams, child = q.get('sub');
    if (!child) return page();
    const before = q.get('before');
    const size = child === 'replace' ? (before === '60' ? 1.9 : 1.5) : 1.55;
    return {...page(), cursor: before === null ? '120' : before === '120' ? '60' : '0', has_earlier: before !== '60',
      turns: [{id: `${child}-${before}`, role: 'assistant', blocks: [{kind: 'text', md: 'x'.repeat(Math.floor(size * 1024 * 1024))}]}]};
  });
  const parentA = h.acquire('a'), parentB = h.acquire('b'); await settle();
  const a = h.store.peek('a'), b = h.store.peek('b');
  const close = a.acquireSubagent('replace'), others: (() => void)[] = [];
  try {
    await a.loadSubagent('replace');
    for (let i = 0; i < 4; i++) {others.push(b.acquireSubagent(`other${i}`)); await b.loadSubagent(`other${i}`);}
    const charged = a.retainedBytes + b.retainedBytes;
    assert.ok(charged > 30 * 1024 * 1024 && charged < 32 * 1024 * 1024);
    await a.loadSubagentEarlier('replace');
    assert.equal(a.subagents.replace.error, null, 'replacement credits the previous page instead of double-counting it');
    assert.equal(a.subagents.replace.turns[0].id, 'replace-120');
    await a.loadSubagentEarlier('replace');
    assert.match(a.subagents.replace.error, /Close another expanded subagent/);
    assert.equal(a.subagents.replace.turns[0].id, 'replace-120', 'a rejected page keeps the existing readable page');
    assert.ok(a.retainedBytes + b.retainedBytes < 32 * 1024 * 1024);
    others.shift()!(); await a.loadSubagent('replace');
    assert.equal(a.subagents.replace.error, null);
    assert.equal(a.subagents.replace.turns[0].id, 'replace-60', 'retry reaches the refused earlier page');
  } finally {close(); others.forEach(release => release()); parentA(); parentB();}
});

test('adaptive child page reduction keeps every older and newer turn reachable', async () => {
  const limits: number[] = [], text = 'x'.repeat(100 * 1024);
  const h = setup(async url => {
    const q = new URL(url, 'http://fixture').searchParams;
    if (!q.has('sub')) return page();
    const limit = Number(q.get('limit')); limits.push(limit);
    const end = q.has('before') ? Number(q.get('before')) : 180, start = Math.max(0, end - limit);
    return {...page(), cursor: String(start), has_earlier: start > 0,
      turns: Array.from({length: end - start}, (_, i) => ({id: String(start + i), role: 'user', blocks: [{kind: 'text', md: text}]}))};
  });
  const parent = h.acquire('a'); await settle(); const c = h.store.peek('a'), close = c.acquireSubagent('adaptive');
  const visited = new Set<string>(), reverseVisited = new Set<string>();
  const inspect = (seen: Set<string>) => {
    const body = c.subagents.adaptive;
    assert.equal(body.error, null);
    assert.ok(body.turns.length > 0 && body.turns.length <= 20);
    assert.ok(JSON.stringify(body.turns).length <= 2 * 1024 * 1024, 'visible payload stays bounded on every page');
    body.turns.forEach((t: any) => seen.add(t.id));
  };
  try {
    await c.loadSubagent('adaptive');
    let pages = 0;
    while (true) {
      assert.ok(++pages <= 180, 'earlier cursors must make finite progress');
      inspect(visited);
      if (!c.subagents.adaptive.has_earlier) break;
      await c.loadSubagentEarlier('adaptive');
    }
    const expectedIds = Array.from({length: 180}, (_, i) => String(i));
    assert.deepEqual([...visited].sort((a, b) => Number(a) - Number(b)), expectedIds);
    assert.equal(c.subagents.adaptive.turns[0].id, '0');
    inspect(reverseVisited);
    for (let i = 1; i < pages; i++) {
      assert.equal(c.subagents.adaptive.has_later, true);
      await c.loadSubagentLater('adaptive');
      inspect(reverseVisited);
    }
    assert.deepEqual([...reverseVisited].sort((a, b) => Number(a) - Number(b)), expectedIds);
    assert.equal(c.subagents.adaptive.turns.at(-1).id, '179');
    assert.equal(c.subagents.adaptive.has_later, false);
    assert.deepEqual(limits.slice(0, 3), [60, 30, 15]);
  } finally {close(); parent();}
});

test('a late collapsed read cannot publish over or release its reopened child reservation', async () => {
  const pending: ((value: any) => void)[] = [];
  const h = setup(url => url.includes('sub=') ? new Promise(resolve => pending.push(resolve)) : Promise.resolve(page()));
  const parent = h.acquire('a'); await settle(); const c = h.store.peek('a');
  const oldClose = c.acquireSubagent('same'), oldRead = c.loadSubagent('same');
  oldClose();
  const freshClose = c.acquireSubagent('same'), freshRead = c.loadSubagent('same');
  const others = Array.from({length: 7}, (_, i) => c.acquireSubagent(`other${i}`));
  const otherReads = others.map((_, i) => c.loadSubagent(`other${i}`));
  const ninthClose = c.acquireSubagent('ninth');
  try {
    pending[0]({...page(), turns: [{id: 'stale', blocks: [], role: 'user'}]}); await oldRead;
    assert.equal(c.subagents.same.loading, true);
    assert.equal(c.subagents.same.turns.length, 0);
    await c.loadSubagent('ninth');
    assert.equal(pending.length, 9, 'old finally must not free the replacement reservation');
    assert.match(c.subagents.ninth.error, /Close another expanded subagent/);
    pending[1]({...page(), turns: [{id: 'fresh', blocks: [], role: 'user'}]}); await freshRead;
    assert.equal(c.subagents.same.turns[0].id, 'fresh');
  } finally {
    freshClose(); others.forEach(release => release()); ninthClose(); parent();
    pending.forEach(resolve => resolve(page())); await Promise.all([oldRead, freshRead, ...otherReads]);
  }
});
