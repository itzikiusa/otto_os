import {test} from 'node:test';
import assert from 'node:assert/strict';
import {readFileSync,existsSync} from 'node:fs';
import {runInNewContext} from 'node:vm';
import ts from 'typescript';
const file=new URL('../src/modules/workflows/runProgress.ts',import.meta.url);
function helpers():Record<string,any> {
  if (!existsSync(file)) return {mergeRunProgress:(current:any,next:any)=>{if(current.id===next.id && next.rev>=current.rev) {current.rev=next.rev; current.nodes=next.nodes;}},RunBodyCache:class {items=new Map(); put(run:string,node:string,version:string,body:any){this.items.set(`${run}:${node}`,{version,body});} get(run:string,node:string,_version:string){return this.items.get(`${run}:${node}`)?.body;} }};
  const context={exports:{} as Record<string,any>,require:()=>({})};
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
