import {test} from 'node:test';
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import {runInNewContext} from 'node:vm';
import ts from 'typescript';
import {componentFunctions} from './componentFunctions.ts';
import {deferred,loadSource} from './sourceHarness.ts';

function handlers(path: string, state: Record<string, any>) {
  const url=new URL(`../src/${path}`,import.meta.url);
  const source=readFileSync(url,'utf8').split('<script lang="ts">')[1].split('</script>')[0];
  const ast=ts.createSourceFile('form.ts',source,ts.ScriptTarget.Latest,true);
  const names=ast.statements.filter(ts.isFunctionDeclaration).map(n=>n.name!.text);
  const guards:((to:string)=>boolean|Promise<boolean>)[]=[];
  const disposals:(()=>void)[]=[];
  const ctx=componentFunctions(url,names,{...state,$state:Object.assign((v:any)=>v,{snapshot:(v:any)=>v}),
    router:{guard:(fn:any)=>{guards.push(fn);return ()=>{};},replace(){}},
    onDestroy:(fn:()=>void)=>disposals.push(fn),untrack:(fn:()=>any)=>fn(),
    $effect:(fn:()=>void)=>fn(),
    guardUnsaved:(dirty:()=>boolean)=>{guards.push(async()=>!dirty()||await state.confirmer.ask());return ()=>{};},
  });
  // Execute the production guard registrations, not unrelated loading effects.
  const registrations=ast.statements.filter(n=>ts.isExpressionStatement(n)&&/(?:\$effect\([\s\S]*(?:router\.guard|guardUnsaved)|^onDestroy\()/.test(n.getText(ast))).map(n=>n.getText(ast)).join('\n');
  runInNewContext(ts.transpileModule(registrations,{compilerOptions:{target:ts.ScriptTarget.ES2022}}).outputText,ctx);
  return {ctx,dispose:()=>disposals.forEach(fn=>fn()),mayLeave:async(to='agents')=>{for(const guard of guards) if(!await guard(to))return false;return true;}};
}
function scheduled(create=false) {
  const calls:{kind:string;id:string;body:any;result:ReturnType<typeof deferred<any>>}[]=[];
  let confirm=false,prompts=0;
  const form=handlers('modules/scheduled-tasks/ScheduledTasksPage.svelte',{
    creating:create,editId:create?null:'task-A',busy:false,error:'',formSnapshot:'',originalDest:'',
    fName:'name A',fPrompt:'prompt A',fSkill:'',fKind:'agent_prompt',fProvider:'claude',fModel:'',fWorkflowId:'',fCadence:'interval',
    fEveryMin:60,fAt:'03:00',fWeekday:0,fCronExpr:'0 9 * * 1',fRunAt:'',fTimezone:'UTC',fSandbox:'none',fMaxRetries:0,
    fNotifyOnChange:false,fAttachProof:false,fDestType:'none',fChatId:'',fEmailTo:'',fUrl:'',fEnabled:true,fCwd:'/tmp',
    ws:{currentId:'ws-A'},tzOk:true,cronFieldCount:5,browserTz:'UTC',defaultAgentProvider:()=> 'claude',
    formGeneration:0,formWorkspaceId:'ws-A',alive:true,
    confirmer:{ask:async()=>{prompts++;return confirm;}},confirmOutward:async()=>true,toasts:{success(){},error(){}},
    fieldError:null,intervalError:'',
    loadErrorText:(e:unknown)=>e instanceof Error?e.message:String(e),toastError(){},
    scheduledTasks:{
      update:(id:string,body:any)=>{const result=deferred<any>();calls.push({kind:'patch',id,body,result});return result.promise;},
      create:(id:string,body:any)=>{const result=deferred<any>();calls.push({kind:'create',id,body,result});return result.promise;},
    },
  });
  form.ctx.formSnapshot=form.ctx.formState();
  return {...form,calls,setConfirm:(value:boolean)=>confirm=value,prompts:()=>prompts};
}
test('scheduled save retains fields typed during PATCH and saves again to the same task',async()=>{
  const h=scheduled();h.ctx.fName='submitted';const saving=h.ctx.save();
  h.ctx.fName='newer';h.ctx.fPrompt='newer prompt';h.ctx.fEveryMin=120;
  h.calls[0].result.resolve({id:'task-A',...h.calls[0].body});await saving;
  assert.equal(h.ctx.editId,'task-A');assert.equal(h.ctx.fName,'newer');assert.notEqual(h.ctx.formSnapshot,h.ctx.formState());
  const again=h.ctx.save();assert.equal(h.calls[1].kind,'patch');assert.equal(h.calls[1].id,'task-A');assert.equal(h.calls[1].body.prompt,'newer prompt');
  h.calls[1].result.resolve({id:'task-A',...h.calls[1].body});await again;assert.equal(h.ctx.editId,null);
});
test('scheduled create adopts its new ID while preserving post-submit edits',async()=>{
  const h=scheduled(true);const creating=h.ctx.save();h.ctx.fName='newer';
  h.calls[0].result.resolve({id:'created-task',...h.calls[0].body});await creating;
  assert.equal(h.ctx.editId,'created-task');assert.equal(h.ctx.fName,'newer');
  const again=h.ctx.save();assert.equal(h.calls[1].kind,'patch');assert.equal(h.calls[1].id,'created-task');
  h.calls[1].result.resolve({id:'created-task',...h.calls[1].body});await again;
});
test('scheduled failed save retains all newer fields',async()=>{
  const h=scheduled();const saving=h.ctx.save();h.ctx.fPrompt='keep';h.ctx.fEveryMin=125;
  h.calls[0].result.reject(Error('offline'));await saving;
  assert.equal(h.ctx.editId,'task-A');assert.equal(h.ctx.fPrompt,'keep');assert.equal(h.ctx.fEveryMin,125);assert.match(h.ctx.error,/offline/);
});
for(const to of ['agents',''])test(`scheduled ${to||'workspace'} navigation uses the form leave decision`,async()=>{
  const h=scheduled();h.ctx.fPrompt='dirty';h.ctx.fEveryMin=90;
  assert.equal(await h.mayLeave(to),false,'Keep editing must reject navigation');assert.equal(h.ctx.fPrompt,'dirty');assert.equal(h.ctx.fEveryMin,90);assert.equal(h.calls.length,0);
  h.setConfirm(true);assert.equal(await h.mayLeave(to),true);assert.equal(h.prompts(),2);
});
test('scheduled clean navigation does not ask, late save cannot close replacement form',async()=>{
  const h=scheduled();assert.equal(await h.mayLeave(),true);assert.equal(h.prompts(),0);
  h.ctx.editId='task-A';const saving=h.ctx.save();h.ctx.ws.currentId='ws-B';h.ctx.editId='task-B';h.ctx.fName='B';h.ctx.formGeneration++;
  h.calls[0].result.resolve({id:'task-A',...h.calls[0].body});await saving;
  assert.equal(h.ctx.editId,'task-B');assert.equal(h.ctx.fName,'B');
});
// The store must not reload an old workspace after its form has been left.
test('scheduled create completion cannot replace another workspace list',async()=>{
  const created=deferred<any>();const lists:string[]=[];
  const {scheduledTasks:store}=loadSource(new URL('../src/lib/stores/scheduledTasks.svelte.ts',import.meta.url),{
    '../api/scheduledTasks':{scheduledTasksApi:{create:()=>created.promise,list:async(id:string)=>{lists.push(id);return [{id:`task-${id}`}];}}},
    '../loadError':{loadErrorText:String},'../lazyModule':{announceModule(){}},
  });
  await store.loadList('A');const saving=store.create('A',{});await store.loadList('B');
  created.resolve({id:'created-A'});await saving;
  assert.equal(store.list[0].id,'task-B');assert.equal(lists.join(','),'A,B');
});

function scheduledStoreLoads() {
  const requests:{id:string;result:ReturnType<typeof deferred<any>>}[]=[];
  const {scheduledTasks:store}=loadSource(new URL('../src/lib/stores/scheduledTasks.svelte.ts',import.meta.url),{
    '../api/scheduledTasks':{scheduledTasksApi:{list:(id:string)=>{const result=deferred<any>();requests.push({id,result});return result.promise;}}},
    '../loadError':{loadErrorText:String},'../lazyModule':{announceModule(){}},
  });
  return {store,requests};
}
for(const staleFails of [false,true])test(`scheduled A-B-A ignores old A ${staleFails?'failure':'success'} and finally`,async()=>{
  const {store,requests}=scheduledStoreLoads();
  const old=store.loadList('A');const other=store.loadList('B');requests[1].result.resolve([{id:'B'}]);await other;
  const latest=store.loadList('A');
  if(staleFails)requests[0].result.reject(Error('old failure'));else requests[0].result.resolve([{id:'old A'}]);await old;
  assert.equal(store.loadingList,true,'old completion must not clear current loading state');
  assert.equal(store.listError,null,'old failure must not become the current error');
  assert.equal(store.list.length,0,'old A rows must not replace the current A request');
  requests[2].result.resolve([{id:'new A'}]);await latest;
  assert.equal(store.list[0].id,'new A');assert.equal(store.loadingList,false);
});
test('scheduled reversed same-workspace load cannot replace the latest success',async()=>{
  const {store,requests}=scheduledStoreLoads();const older=store.loadList('A');const newer=store.loadList('A');
  requests[1].result.resolve([{id:'new'}]);await newer;requests[0].result.resolve([{id:'old'}]);await older;
  assert.equal(store.list[0].id,'new');
});

function loopStore() {
  const requests:{path:string;result:ReturnType<typeof deferred<any>>}[]=[];
  const {loops:store}=loadSource(new URL('../src/lib/stores/loops.svelte.ts',import.meta.url),{
    '../api/client':{api:{get:(path:string)=>{const result=deferred<any>();requests.push({path,result});return result.promise;}}},
    '../loadError':{loadErrorText:String},
  });
  async function load(id:string) {const pending=store.loadList(id);requests.at(-1)!.result.resolve([{id:`loop-${id}`}]);await pending;}
  return {store,requests,load};
}
for(const fails of [false,true])test(`goal A-B-A ignores old list ${fails?'failure':'success'} and finally`,async()=>{
  const h=loopStore();const old=h.store.loadList('A');await h.load('B');const current=h.store.loadList('A');
  if(fails)h.requests[0].result.reject(Error('old failure'));else h.requests[0].result.resolve([{id:'old-A'}]);await old;
  assert.equal(h.store.loadingList,true);assert.equal(h.store.listError,null);assert.equal(h.store.list.length,0);
  h.requests.at(-1)!.result.resolve([{id:'new-A'}]);await current;assert.equal(h.store.list[0].id,'new-A');
});
test('goal reversed same-workspace lists preserve the newer result',async()=>{
  const h=loopStore();const old=h.store.loadList('A');const current=h.store.loadList('A');
  h.requests[1].result.resolve([{id:'new'}]);await current;h.requests[0].result.resolve([{id:'old'}]);await old;
  assert.equal(h.store.list[0].id,'new');
});
