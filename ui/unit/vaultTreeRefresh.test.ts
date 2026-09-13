import {test} from 'node:test';
import assert from 'node:assert/strict';
import {refreshVisibleTree, type RefreshNode} from '../src/modules/vault/treeRefresh.ts';
type Entry={path:string;kind:string};
const folder=(path:string,open=true):RefreshNode<Entry>=>({entry:{path,kind:'dir'},depth:0,open,loaded:true,loading:false,children:[]});

test('visible directory requests are bounded to four and old nodes are immutable',async()=>{
  const previous=Array.from({length:12},(_,i)=>folder(`dir${i}`));
  const waiters:Array<()=>void>=[]; let active=0,peak=0;
  const result=refreshVisibleTree(previous,async path=>{
    if(!path)return previous.map(n=>n.entry);
    active++;peak=Math.max(peak,active);
    await new Promise<void>(resolve=>waiters.push(resolve));
    active--;return [{path:`${path}/new.md`,kind:'note'}];
  });
  for(let round=0;round<3;round++){
    for(let i=0;i<20&&waiters.length<4;i++)await Promise.resolve();
    assert.equal(waiters.length,4);waiters.splice(0).forEach(resolve=>resolve());
  }
  const next=await result;
  assert.equal(peak,4); assert.equal(next[0].children.length,1);
  assert.equal(previous[0].children.length,0);
});

test('stale navigation never mutates a previously shown nested node',async()=>{
  const original=folder('open'); let current=true; let finish:(value:Entry[])=>void=()=>{};
  const result=refreshVisibleTree([original],async path=>path?await new Promise<Entry[]>(resolve=>{finish=resolve;}):[original.entry],()=>current);
  for(let i=0;i<10;i++)await Promise.resolve();
  current=false;finish([{path:'open/stale.md',kind:'note'}]);await result;
  assert.deepEqual(original.children,[]);
});

test('a stale collapsed branch refreshes preserved open descendants on reopening',async()=>{
  const nested=folder('closed/nested'); nested.depth=1;
  const calls:string[]=[];
  const result=await refreshVisibleTree([nested],async path=>{
    calls.push(path);return path==='closed'?[nested.entry]:[{path:'closed/nested/new.md',kind:'note'}];
  },()=>true,'closed',1);
  assert.deepEqual(calls,['closed','closed/nested']);assert.equal(result[0].children.length,1);
});
