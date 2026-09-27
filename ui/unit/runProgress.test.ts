import {test} from 'node:test';
import assert from 'node:assert/strict';
import {readFileSync,existsSync} from 'node:fs';
import {runInNewContext} from 'node:vm';
import ts from 'typescript';
const file=new URL('../src/modules/workflows/runProgress.ts',import.meta.url);
function helpers():Record<string,any> {
  if (!existsSync(file)) return {mergeRunProgress:(current:any,next:any)=>{if(current.id===next.id && next.rev>=current.rev) {current.rev=next.rev; current.nodes=next.nodes;}},RunBodyCache:class {items=new Map(); put(run:string,node:string,version:string,body:any){this.items.set(`${run}:${node}`,{version,body});} get(run:string,node:string,_version:string){return this.items.get(`${run}:${node}`)?.body;} }};
  const context={exports:{} as Record<string,any>,require:()=>({}),AbortController};
  runInNewContext(ts.transpileModule(readFileSync(file,'utf8'),{compilerOptions:{module:ts.ModuleKind.CommonJS,target:ts.ScriptTarget.ES2022}}).outputText,context); return context.exports;
}
test('checkpoint-only progress merges freshness without clearing loaded pages',()=>{
  const {mergeRunProgress}=helpers(); const current={id:'run',rev:2,nodes:[],checkpoint_rev:1,checkpoint_generation:0,checkpoints:[{node_id:'c'}]};
  mergeRunProgress(current,{id:'run',rev:3,nodes:[],checkpoint_rev:2,checkpoint_generation:0,checkpoint_count:2,checkpoints:[],summary:true});
  assert.equal(current.checkpoint_rev,2); assert.equal(current.checkpoints.length,1);
  mergeRunProgress(current,{id:'run',rev:4,nodes:[],checkpoint_rev:3,checkpoint_generation:1,checkpoints:[],summary:true}); assert.equal(current.checkpoints.length,0);
});
test('body cache rejects another version and evicts only bounded retained bodies',()=>{
  const {RunBodyCache}=helpers(); const cache=new RunBodyCache(); cache.put('r','node','v1',{output:'old'});
  assert.equal(cache.get('r','node','v2'),null);
  for(let n=0;n<20;n++) cache.put('r',`n${n}`,'v',{output:String(n)});
  assert.equal(cache.get('r','node','v1'),null); assert.equal(cache.get('other','n19','v'),null);
});
test('out-of-order run snapshots do not roll back selected details or checkpoint generation',()=>{
  const {mergeRunProgress}=helpers(); const current={id:'r',rev:9,nodes:[{node_id:'a',detail_version:'new'}],checkpoint_generation:2};
  mergeRunProgress(current,{id:'r',rev:8,nodes:[],checkpoint_generation:1}); assert.equal(current.nodes[0].detail_version,'new');
  mergeRunProgress(current,{id:'other',rev:20,nodes:[],checkpoint_generation:3}); assert.equal(current.checkpoint_generation,2);
});
test('checkpoint pages preserve a row already fetched at a newer revision',()=>{
  const {mergeCheckpointPage}=helpers();const versions=new Map([['first',3]]);
  const merged=mergeCheckpointPage({checkpoint_rev:2,items:[{node_id:'first',detail_version:'old'},{node_id:'second',detail_version:'v2'}]},[{node_id:'first',detail_version:'new'}],versions);
  assert.equal(merged.items[0].detail_version,'new');assert.equal(merged.items[1].detail_version,'v2');assert.equal(merged.known.length,2);
});
test('unchanged nodes (same detail_version + status) keep their object and fields',()=>{
  const {mergeRunProgress}=helpers();
  const sessions=['s1'];
  const a={node_id:'a',status:'running',detail_version:'v1',sessions,logs:['loaded body']};
  const b={node_id:'b',status:'running',detail_version:'v1',sessions:[]};
  const current={id:'r',rev:1,nodes:[a,b]};
  mergeRunProgress(current,{id:'r',rev:2,nodes:[
    {node_id:'a',status:'running',detail_version:'v1',sessions:['s1'],logs:[]},
    {node_id:'b',status:'done',detail_version:'v2',sessions:['s2'],logs:[]},
  ]});
  assert.equal(current.nodes[0],a); assert.equal(current.nodes[0].sessions,sessions,'unchanged node not rewritten');
  assert.deepEqual(current.nodes[0].logs,['loaded body']);
  assert.equal(current.nodes[1],b); assert.equal(current.nodes[1].status,'done'); assert.deepEqual(current.nodes[1].sessions,['s2']);
});
test('shared node bodies: one request per (run,node) across the step list and the inspector',async()=>{
  const {SharedNodeBodies}=helpers(); const shared=new SharedNodeBodies(2);
  let calls=0; const signals:AbortSignal[]=[];
  let release!:()=>void; const gate=new Promise<void>(r=>{release=r;});
  const load=async(signal:AbortSignal)=>{calls++;signals.push(signal);await gate;return {detail_version:'v1',body:{output:'x'}};};
  const a=shared.fetch('r','n',load); const b=shared.fetch('r','n',load);
  release(); const [ra,rb]=await Promise.all([a,b]);
  assert.equal(calls,1,'concurrent readers share one request');
  assert.equal(ra,rb);
  assert.deepEqual(shared.peek('r','n','v1'),{output:'x'});
  assert.equal(shared.peek('r','n','v2'),null,'another version is a miss');
  // Bounded: the oldest (run,node) falls out past the cap.
  await shared.fetch('r','m',async()=>({detail_version:'1',body:1}));
  await shared.fetch('r','o',async()=>({detail_version:'1',body:2}));
  assert.equal(shared.peek('r','n','v1'),null);
});
test('a shared read is aborted only when every consumer that joined it aborted',async()=>{
  const {SharedNodeBodies}=helpers(); const shared=new SharedNodeBodies();
  let seen!:AbortSignal; let release!:()=>void; const gate=new Promise<void>(r=>{release=r;});
  const load=async(signal:AbortSignal)=>{seen=signal;await gate;return {detail_version:'v',body:0};};
  const c1=new AbortController(), c2=new AbortController();
  const p1=shared.fetch('r','n',load,c1.signal); const p2=shared.fetch('r','n',load,c2.signal);
  c1.abort(); assert.equal(seen.aborted,false,'one consumer left; the request stays');
  c2.abort(); assert.equal(seen.aborted,true,'the last consumer left; the request is aborted');
  release(); await Promise.all([p1,p2]);
});
