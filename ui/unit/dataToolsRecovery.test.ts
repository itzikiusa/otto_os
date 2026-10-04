import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { runInNewContext } from 'node:vm';
import ts from 'typescript';
import { SchemaVersionCache } from '../src/modules/brokers/schemaVersionCache.ts';

// Execute the actual component functions with controlled asynchronous boundaries.
function componentFunctions(path: string, names: string[], state: Record<string, any>) {
  const text = readFileSync(new URL(path, import.meta.url), 'utf8').split('<script lang="ts">')[1].split('</script>')[0];
  const file = ts.createSourceFile('component.ts', text, ts.ScriptTarget.Latest, true, ts.ScriptKind.TS);
  const selected = file.statements.filter(node => ts.isFunctionDeclaration(node) && node.name && names.includes(node.name.text)).map(node => node.getText(file)).join('\n');
  const context = {exports:{}, ...state};
  runInNewContext(ts.transpileModule(selected, {compilerOptions:{module:ts.ModuleKind.CommonJS,target:ts.ScriptTarget.ES2022}}).outputText, context);
  return context as Record<string, any>;
}

test('scratch close cancel retains the exact tab; discard resolves its stable id', async () => {
  let answer = false, closed = -1;
  const draft = {tabId:'scratch',body:'unsent',post_response_script:'test'};
  const context = componentFunctions('../src/modules/api/ApiPage.svelte', ['closeRequestTab'], {
    apiClient:{tabs:[draft], isDirty:()=>true, tabLabel:()=> 'scratch',closeTab:(i:number)=>{closed=i;}},
    confirmer:{ask:async()=>answer},
  });
  await context.closeRequestTab(0);assert.equal(closed,-1);
  answer=true;await context.closeRequestTab(0);assert.equal(closed,0);
});

test('automation cancel and failed save retain steps; successful save or discard permits leave', async () => {
  let choice: string | null = null, saved = false;
  const context = componentFunctions('../src/modules/api/AutomationEditor.svelte', ['approveLeave','save'], {
    dirty:true,leavePending:null,steps:[{request_id:'r',assertions:[{kind:'status',value:'201'}],extract:[]}],
    automation:{id:'a',name:'A'},canEdit:true,
    confirmer:{choose:async()=>({value:choice})},
    apiClient:{saveAutomation:async()=>saved?{name:'A'}:null},toasts:{success(){}},
  });
  assert.equal(await context.approveLeave(),false);assert.equal(context.dirty,true);
  choice='save';assert.equal(await context.approveLeave(),false);assert.equal(context.steps[0].assertions[0].value,'201');
  saved=true;assert.equal(await context.approveLeave(),true);assert.equal(context.dirty,false);
  context.dirty=true;choice='discard';assert.equal(await context.approveLeave(),true);
});

test('automation edits arriving during save keep the editor open', async () => {
  let release!: (value: unknown) => void;
  const pending = new Promise(r=>{release=r;});
  let started!: () => void;
  const saving = new Promise<void>(resolve=>{started=resolve;});
  const context = componentFunctions('../src/modules/api/AutomationEditor.svelte', ['approveLeave','save'], {
    dirty:true,leavePending:null,steps:[{request_id:'r',assertions:[],extract:[]}],automation:{id:'a',name:'A'},canEdit:true,
    confirmer:{choose:async()=>({value:'save'})},apiClient:{saveAutomation:()=>{started();return pending;}},toasts:{success(){}},
  });
  const leaving=context.approveLeave();await saving;
  context.steps[0].request_id='edited-while-saving';release({name:'A'});
  assert.equal(await leaving,false);assert.equal(context.dirty,true);
});

test('tail failures retain the buffer and cursors, clear new count and recover on empty success', async () => {
  let fail=true;
  const original={messages:[{partition:0,offset:10}],partitions:[],truncated:false};
  const context=componentFunctions('../src/modules/brokers/TopicDetail.svelte', ['consumeWithTail'], {
    tailOffsets:new Map([[0,10]]),tailAdded:4,tailError:null,tailLastSuccess:123,tailSeq:0,
    consuming:false,partition:'',decode:true,maskPayloads:false,result:original,
    consumeUrl:()=>'/consume',loadErrorText:(e:Error)=>e.message,
    api:{post:async(_url:string,req:any)=>{assert.equal(req.start_offsets[0].offset,11);if(fail)throw Error('broker down');return {messages:[],partitions:[]};}},
  });
  await context.consumeWithTail(true);
  assert.equal(context.result,original);assert.equal(context.tailOffsets.get(0),10);
  assert.equal(context.tailAdded,0);assert.equal(context.tailError,'broker down');assert.equal(context.consuming,false);
  fail=false;await context.consumeWithTail(true);
  assert.equal(context.tailError,null);assert.ok(context.tailLastSuccess>123);assert.equal(context.result,original);
});

test('canceled automation navigation keeps the selected editor and request tab', async () => {
  let allow=false;
  const initial={kind:'automation',id:'a'};
  let switched=-1;
  const context=componentFunctions('../src/modules/api/ApiPage.svelte',['changeView','switchRequestTab'],{
    view:initial,viewTransition:0,phonePane:'main',automationEditor:{approveLeave:async()=>allow},
    apiClient:{tabs:[{tabId:'request'}],switchTab:(i:number)=>{switched=i;}},
  });
  await context.switchRequestTab(0);assert.equal(context.view,initial);assert.equal(switched,-1);
  allow=true;await context.switchRequestTab(0);assert.equal(context.view.kind,'request');assert.equal(switched,0);
});

test('a late tail response cannot revive another topic buffer or failure state', async () => {
  let release!: (value: unknown) => void;
  const pending=new Promise(resolve=>{release=resolve;});
  let url='/old';
  const context=componentFunctions('../src/modules/brokers/TopicDetail.svelte',['consumeWithTail'],{
    tailOffsets:new Map([[0,10]]),tailAdded:0,tailError:null,tailLastSuccess:null,tailSeq:0,
    consuming:false,partition:'',decode:true,maskPayloads:false,result:null,
    consumeUrl:()=>url,loadErrorText:(e:Error)=>e.message,api:{post:()=>pending},
  });
  const old=context.consumeWithTail(true);url='/new';
  release({messages:[{partition:0,offset:11}],partitions:[]});await old;
  assert.equal(context.result,null);assert.equal(context.tailLastSuccess,null);assert.equal(context.tailError,null);
});

test('schema history fetches two selected bodies, caches revisits and aborts superseded detail', async () => {
  const gets: string[]=[];
  let pendingSignal: AbortSignal | undefined;
  const cache=new SchemaVersionCache();
  const context=componentFunctions('../src/modules/brokers/SchemaVersionsPanel.svelte', ['loadVersions','cancelLoads','selectVersion'], {
    AbortController,generation:0,listController:null,detailControllers:{A:null,B:null},cache,
    loading:false,loadError:null,detailErrors:{A:null,B:null},versions:[],diffA:null,diffB:null,
    selectedA:null,selectedB:null,showDiff:false,compatResult:null,
    versionBase:()=>'/versions',loadErrorText:(e:Error)=>e.message,
    api:{get:async(url:string,signal:AbortSignal)=>{
      gets.push(url);
      if(url==='/versions')return Array.from({length:100},(_,i)=>({version:i+1}));
      const version=Number(url.split('/').pop());
      if(version===1){pendingSignal=signal;return new Promise((_resolve,reject)=>signal.addEventListener('abort',()=>reject(new DOMException('canceled','AbortError'))));}
      return {version,id:version,subject:'s',schema:'{}',schema_type:'JSON'};
    }},
  });
  context.loadVersions();await new Promise(setImmediate);
  assert.deepEqual(gets,['/versions','/versions/100','/versions/99']);
  assert.equal(context.diffA.version,99);assert.equal(context.diffB.version,100);
  await context.selectVersion('A',100);assert.equal(gets.length,3,'cached selected body');
  const old=context.selectVersion('A',1);
  await context.selectVersion('A',99);await old;
  assert.equal(pendingSignal?.aborted,true);assert.equal(context.diffA.version,99);
  assert.equal(context.detailErrors.A,null);
});
