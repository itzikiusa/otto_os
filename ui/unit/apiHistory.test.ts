import {test} from 'node:test';
import assert from 'node:assert/strict';
import {HistoryRefresh, HistoryDetail} from '../src/lib/stores/apiHistory.ts';
function deferred<T>() { let resolve!: (value:T)=>void; const promise=new Promise<T>(r=>{resolve=r;}); return {promise,resolve}; }
function setup() {
  let workspace:string|null='A'; const scheduled:Array<()=>void>=[]; const calls:Array<{ws:string,signal:AbortSignal,result:ReturnType<typeof deferred<Array<{id:string}>>>}>=[]; const published:Array<{id:string}[]>=[];
  const refresh=new HistoryRefresh({workspace:()=>workspace, fetch:(ws:string,signal:AbortSignal)=>{const result=deferred<Array<{id:string}>>();calls.push({ws,signal,result});return result.promise;}, publish:(rows:Array<{id:string}>)=>published.push(rows), error:()=>{}, schedule:(fn:()=>void)=>{scheduled.push(fn);return ()=>{const n=scheduled.indexOf(fn);if(n>=0)scheduled.splice(n,1);};}});
  return {refresh,calls,published,scheduled,workspace:(v:string)=>{workspace=v;}, flush:()=>{scheduled.shift()?.();}};
}
async function settle(){await Promise.resolve();await Promise.resolve();await Promise.resolve();}
test('history direct and event invalidations share one snapshot, including duplicate event during fetch',async()=>{
 const f=setup(); const direct=f.refresh.request(); const event=f.refresh.request('one');assert.equal(f.scheduled.length,1);f.flush();assert.equal(f.calls.length,1);
 const duplicate=f.refresh.request('one');f.calls[0].result.resolve([{id:'one'}]);await settle();await Promise.all([direct,event,duplicate]);assert.equal(f.scheduled.length,0);assert.equal(f.published.length,1);
 await f.refresh.request('one');assert.equal(f.scheduled.length,0);
});
test('history append after fetch began produces exactly one necessary followup',async()=>{
 const f=setup();const first=f.refresh.request();f.flush();const later=f.refresh.request('two');f.refresh.request('two');f.calls[0].result.resolve([{id:'one'}]);await settle();await first;assert.equal(f.scheduled.length,1);f.flush();assert.equal(f.calls.length,2);f.calls[1].result.resolve([{id:'two'},{id:'one'}]);await settle();await later;assert.equal(f.scheduled.length,0);
});
test('history workspace replacement aborts and ignores old response',async()=>{
 const f=setup();const old=f.refresh.request();f.flush();f.workspace('B');const current=f.refresh.request();assert.equal(f.calls[0].signal.aborted,true);f.flush();f.calls[0].result.resolve([{id:'old'}]);f.calls[1].result.resolve([{id:'new'}]);await settle();await Promise.all([old,current]);assert.deepEqual(f.published,[[{id:'new'}]]);
});
test('history detail cannot overwrite another selection, workspace, tab or edited draft',async()=>{
 let owner={workspace:'A',tab:'tab-a',draft:{body:'original'}}; const pending:Array<ReturnType<typeof deferred<string>>>=[];const signals:AbortSignal[]=[];const published:string[]=[];
 const detail=new HistoryDetail({owner:()=>owner, fetch:(_id:string,signal:AbortSignal)=>{const d=deferred<string>();pending.push(d);signals.push(signal);return d.promise;},publish:(v:string)=>published.push(v),pending:()=>{},error:()=>{}});
 const a=detail.select('a');const b=detail.select('b');assert.equal(signals[0].aborted,true);pending[0].resolve('a');pending[1].resolve('b');await Promise.all([a,b]);assert.deepEqual(published,['b']);
 for(const change of ['draft','tab','workspace']) {const old=detail.select('old');owner=change==='draft'?{...owner,draft:{body:'edited'}}:change==='tab'?{...owner,tab:'tab-b'}:{...owner,workspace:'B'};pending.at(-1)!.resolve('stale');await old;}
 assert.deepEqual(published,['b']);
});
test('detail failure preserves draft and clears only its own pending state',async()=>{
 const errors:string[]=[];const pending:(string|null)[]=[];const owner={workspace:'A',tab:'a',draft:{}};
 const detail=new HistoryDetail({owner:()=>owner,fetch:async()=>{throw new Error('fixture unavailable');},publish:()=>assert.fail('failed detail published'),pending:(id:string|null)=>pending.push(id),error:(error:unknown)=>errors.push(String(error))});
 await detail.select('a');assert.deepEqual(pending,['a',null]);assert.equal(errors.length,1);
});
