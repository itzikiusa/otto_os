import {test} from 'node:test';
import assert from 'node:assert/strict';
import {spliceHistory} from '../src/modules/git/graph-splice.ts';

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
