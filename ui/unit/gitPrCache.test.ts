import {test} from 'node:test';
import assert from 'node:assert/strict';
import {SwrCache, prListCache, prDetailCache, prListKey, prKey, invalidatePr} from '../src/modules/git/pr-cache.ts';

test('SwrCache serves fresh, then stale, then drops past max age',()=>{
  let t=0;
  const c=new SwrCache<number>(8,100,1000,()=>t);
  c.set('a',1);
  assert.deepEqual(c.get('a'),{value:1,fresh:true});
  t=150;
  assert.deepEqual(c.get('a'),{value:1,fresh:false},'stale but still painted');
  t=1200;
  assert.equal(c.get('a'),undefined);
  assert.equal(c.size,0);
});

test('SwrCache is a bounded LRU',()=>{
  const c=new SwrCache<number>(2);
  c.set('a',1);c.set('b',2);
  c.get('a');            // touch a → b is the oldest
  c.set('c',3);
  assert.equal(c.get('b'),undefined);
  assert.equal(c.get('a')?.value,1);
  assert.equal(c.get('c')?.value,3);
});

test('invalidatePr drops the repo lists and only that PR',()=>{
  prListCache.set(prListKey('r1','open',1),{items:[],has_more:false} as never);
  prListCache.set(prListKey('r2','open',1),{items:[],has_more:false} as never);
  prDetailCache.set(prKey('r1',1),{} as never);
  prDetailCache.set(prKey('r1',12),{} as never);
  invalidatePr('r1',1);
  assert.equal(prListCache.get(prListKey('r1','open',1)),undefined);
  assert.ok(prListCache.get(prListKey('r2','open',1)));
  assert.equal(prDetailCache.get(prKey('r1',1)),undefined);
  assert.ok(prDetailCache.get(prKey('r1',12)),'prefix r1|1 must not swallow r1|12');
});
