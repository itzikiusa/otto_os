import {test} from 'node:test';
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import ts from 'typescript';
import {deferred} from './sourceHarness.ts';
import * as helpers from '../src/modules/workflows/runProgress.ts';

test('mounted checkpoint loader shows matching-generation membership while progress keeps advancing',async()=>{
  const source=readFileSync(new URL('../src/modules/workflows/RunSteps.svelte',import.meta.url),'utf8').split('<script lang="ts">')[1].split('</script>')[0];
  const ast=ts.createSourceFile('steps.ts',source,ts.ScriptTarget.Latest,true);
  const fn=ast.statements.find(n=>ts.isFunctionDeclaration(n)&&n.name?.text==='loadCheckpointPage')!;
  const replies=[deferred<any>(),deferred<any>()];let calls=0;
  const compiled=ts.transpileModule(`
    let run={id:'r',summary:true,checkpoint_generation:1,checkpoint_rev:1,checkpoints:[]};
    let checkpointPages=[],checkpointPageIndex=0,checkpointLoading=false,checkpointError='',checkpointRefreshQueued=false,checkpointRequest=0,checkpointOpen=true;
    let loadedCheckpoints=[],checkpointRowVersions=new Map();
    const queueCheckpointRefresh=()=>{};
    ${source.slice(fn.getStart(ast),fn.end)}
    return {load:loadCheckpointPage,run,pages:()=>checkpointPages};
  `,{compilerOptions:{target:ts.ScriptTarget.ES2022}}).outputText;
  const h=new Function('workflowCheckpointPage','mergeCheckpointPage',compiled)(()=>replies[calls++]?.promise??new Promise(()=>{}),(helpers as any).mergeCheckpointPage);
  const first=h.load();h.run.checkpoint_rev=2;replies[0].resolve({items:[{node_id:'first',detail_version:'v1'}],checkpoint_rev:1,generation:1,next_cursor:null});await first;
  assert.equal(h.pages().length,1,'a new progress revision must not hide the first fetched page');
  const second=h.load();h.run.checkpoint_rev=3;replies[1].resolve({items:[{node_id:'first',detail_version:'v2'}],checkpoint_rev:2,generation:1,next_cursor:null});await second;
  assert.equal(h.pages()[0].page.items[0].detail_version,'v2');
});
