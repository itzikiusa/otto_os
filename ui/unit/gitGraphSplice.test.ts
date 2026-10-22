import {test} from 'node:test';
import assert from 'node:assert/strict';
import {spliceHistory} from '../src/modules/git/graph-splice.ts';
import { componentFunctions } from './componentFunctions.ts';

const c=(sha:string)=>({sha} as never as {sha:string});
const list=(...s:string[])=>s.map(c) as never[];
const shas=(l:unknown[]|null)=>l?.map((x)=>(x as {sha:string}).sha).join(',') ?? null;

test('new commits on top splice onto the immutable tail',()=>{
  const old=list('d','c','b','a');
  // page of 3 (the reload limit) after one new commit "e"
  assert.equal(shas(spliceHistory(old,list('e','d','c'))),'e,d,c,b,a');
});

test('page last sha unknown → full reload',()=>{
  assert.equal(spliceHistory(list('d','c','b','a'),list('x','y','z')),null);
});

test('a commit above the boundary vanished (deleted branch) → full reload',()=>{
  // old had "f" (branch tip) above c; the new page no longer holds it
  assert.equal(spliceHistory(list('f','d','c','b','a'),list('e','d','c')),null);
});

test('a page commit already in the old tail (reorder) → full reload',()=>{
  assert.equal(spliceHistory(list('d','c','b','a'),list('b','d','c')),null);
});

test('nothing to splice when the page covers everything held',()=>{
  assert.equal(spliceHistory(list('b','a'),list('c','b','a')),null);
});

test('rewinding a same-name ref outside the fresh prefix invalidates the retained tail', () => {
  const graph = componentFunctions(new URL('../src/modules/git/GraphView.svelte', import.meta.url), ['lostRef'], {
    commits: list('recent-1', 'recent-2', 'old-tip', 'base'), FIRST_PAGE: 2,
    movedRefTips: new Set<string>(), newRefTips: new Set<string>(),
  });
  const refs = (sha: string) => ({ local: [{ name: 'old-branch', sha }], remote: [], tags: [] });
  assert.equal(graph.lostRef(refs('old-tip'), refs('base')), true);
  assert.equal(graph.lostRef(refs('old-tip'), refs('old-tip')), false);
  // Fast growth whose old tip is inside the checked prefix keeps the splice path.
  assert.equal(graph.lostRef(refs('recent-1'), refs('new-tip')), false);
  assert.equal(graph.lostRef({ local: [], remote: [], tags: [{ name: 'v1', sha: 'old-tip' }] },
    { local: [], remote: [], tags: [{ name: 'v1', sha: 'base' }] }), true);
});

test('a moved tip displaced from the fresh page also invalidates a spliced tail', () => {
  const reloads: boolean[] = [];
  const graph = componentFunctions(new URL('../src/modules/git/GraphView.svelte', import.meta.url), ['setRefs', 'lostRef', 'applyGraph'], {
    refs: { local: [{ name: 'side', sha: 'old-tip' }], remote: [], tags: [] }, refsKey: '',
    commits: list('recent', 'old-tip', 'base'), FIRST_PAGE: 2,
    refRemoved: false, movedRefTips: new Set<string>(), newRefTips: new Set<string>(), loadGen: 1,
    reloadGraph: (full: boolean) => { reloads.push(full); },
    setCommits() {}, setHasMore() {}, setStashes() {}, setWorktrees() {},
    graphCache: { set() {} },
    commitsError: null, skipCursor: 3, hasMore: false, refreshFailures: 0, refreshError: null,
  });
  graph.setRefs({ local: [{ name: 'side', sha: 'base' }], remote: [], tags: [] });
  // New unrelated main commits displaced the old tip below the fresh boundary.
  graph.applyGraph({ gen: 1, more: false, commits: list('new', 'recent', 'old-tip', 'base'), want: 2 });
  assert.deepEqual(reloads, [true]);
});

test('a NEW ref whose tip is older than the fresh page re-reads the history in full', () => {
  const reloads: boolean[] = [];
  const newRefTips = new Set<string>();
  const scope = {
    refs: { local: [{ name: 'main', sha: 'recent' }], remote: [], tags: [] }, refsKey: '',
    commits: list('recent', 'mid', 'base'), FIRST_PAGE: 2,
    refRemoved: false, movedRefTips: new Set<string>(), newRefTips, loadGen: 1,
    reloadGraph: (full: boolean) => { reloads.push(full); },
    setCommits() {}, setHasMore() {}, setStashes() {}, setWorktrees() {},
    graphCache: { set() {} },
    commitsError: null, skipCursor: 3, hasMore: false, refreshFailures: 0, refreshError: null,
  };
  const graph = componentFunctions(new URL('../src/modules/git/GraphView.svelte', import.meta.url), ['setRefs', 'lostRef', 'applyGraph'], scope);
  // A fetched branch whose (weeks-old) commits sit below the 2-commit page.
  graph.setRefs({ local: [{ name: 'main', sha: 'recent' }], remote: [{ name: 'origin/old-feature', sha: 'old-feat' }], tags: [] });
  assert.ok(newRefTips.has('old-feat'));
  graph.applyGraph({ gen: 1, more: false, commits: list('recent', 'mid', 'base'), want: 2 });
  assert.deepEqual(reloads, [true]);
});

test('a NEW ref whose tip is inside the fresh page keeps the splice path', () => {
  const reloads: boolean[] = [];
  const newRefTips = new Set<string>();
  const graph = componentFunctions(new URL('../src/modules/git/GraphView.svelte', import.meta.url), ['setRefs', 'lostRef', 'applyGraph'], {
    refs: { local: [{ name: 'main', sha: 'recent' }], remote: [], tags: [] }, refsKey: '',
    commits: list('recent', 'mid', 'base'), FIRST_PAGE: 2,
    refRemoved: false, movedRefTips: new Set<string>(), newRefTips, loadGen: 1,
    reloadGraph: (full: boolean) => { reloads.push(full); },
    setCommits() {}, setHasMore() {}, setStashes() {}, setWorktrees() {},
    graphCache: { set() {} },
    commitsError: null, skipCursor: 3, hasMore: false, refreshFailures: 0, refreshError: null,
  });
  graph.setRefs({ local: [{ name: 'main', sha: 'recent' }, { name: 'topic', sha: 'fresh' }], remote: [], tags: [] });
  graph.applyGraph({ gen: 1, more: false, commits: list('fresh', 'recent', 'mid', 'base'), want: 2 });
  assert.deepEqual(reloads, []);
  assert.equal(newRefTips.size, 0);
});
