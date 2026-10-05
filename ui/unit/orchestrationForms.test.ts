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
const flush=()=>new Promise(resolve=>setImmediate(resolve));
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
function goal() {
  let confirm=false,prompts=0;const result=deferred<any>();const created:string[]=[];
  const form=handlers('modules/loops/GoalDefineForm.svelte',{
    seed:'',mode:'build',allowCommits:false,requireReview:false,sourceLinks:'',selectedSkills:'',repoPath:'',picking:false,feedback:'',
    defining:false,launching:false,draft:null,name:'',maxIterations:5,maxMinutes:30,perPhaseMinutes:10,executorCount:1,
    execProvider:'claude',execModel:'',defProvider:'claude',defModel:'',showAdvanced:false,ws:{currentId:'ws-A'},alive:true,formGeneration:0,
    draftSnapshot:'',initialSnapshot:'',formWorkspaceId:'ws-A',defaultAgentProvider:()=> 'claude',
    confirmer:{ask:async()=>{prompts++;return confirm;}},oncancel(){},oncreated:(id:string)=>created.push(id),toastError(){},
    loops:{create:()=>result.promise},
  });
  form.ctx.initialSnapshot=form.ctx.formState?.() ?? '';
  return {...form,result,created,setConfirm:(v:boolean)=>confirm=v,prompts:()=>prompts};
}
for(const to of ['agents',''])test(`goal ${to||'workspace'} navigation preserves criterion and budget drafts`,async()=>{
  const h=goal();h.ctx.seed='do work';h.ctx.maxMinutes=120;
  h.ctx.draft={definition:{acceptance_criteria:[{id:'c1',text:'edited',verify:'proof'}]}};
  assert.equal(await h.mayLeave(to),false);assert.equal(h.ctx.maxMinutes,120);assert.equal(h.ctx.draft.definition.acceptance_criteria[0].text,'edited');
  h.setConfirm(true);assert.equal(await h.mayLeave(to),true);assert.equal(h.prompts(),2);
});
test('goal budget-only changes are a draft and clean navigation does not ask',async()=>{
  const h=goal();assert.equal(await h.mayLeave(),true);assert.equal(h.prompts(),0);
  // Clean accepted leave unmounts that form; a later edit belongs to a new one.
  const edited=goal();edited.ctx.maxMinutes=120;assert.equal(await edited.mayLeave(),false);
});
test('late goal launch cannot redirect a replacement workspace',async()=>{
  const h=goal();h.ctx.name='Goal';h.ctx.repoPath='/tmp';h.ctx.seed='work';
  h.ctx.draft={definition:{title:'Goal',acceptance_criteria:[{id:'c1',text:'done',verify:'proof'}]},suggested_config:{executors:[]}};
  const launch=h.ctx.launch();await flush();h.ctx.ws.currentId='ws-B';h.result.resolve({id:'old-loop'});await launch;
  assert.equal(h.created.length,0,'departed form must not navigate to the old launch');
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
  const created=deferred<any>();
  const {loops:store}=loadSource(new URL('../src/lib/stores/loops.svelte.ts',import.meta.url),{
    '../api/client':{api:{get:(path:string)=>{const result=deferred<any>();requests.push({path,result});return result.promise;},post:()=>created.promise}},
    '../loadError':{loadErrorText:String},'../live':{liveQuery(){}},'../lazyModule':{announceModule(){}},
  });
  async function load(id:string) {const pending=store.loadList(id);requests.at(-1)!.result.resolve([{id:`loop-${id}`}]);await pending;}
  return {store,requests,created,load};
}
for(const returnToA of [false,true])test(`goal create after A-B${returnToA?'-A':''} cannot reload a departed context`,async()=>{
  const h=loopStore();await h.load('A');const creating=h.store.create('A',{});await h.load('B');if(returnToA)await h.load('A');
  const before=h.requests.length;h.created.resolve({id:'created-A'});await flush();
  // Settle any forbidden refresh so a failing implementation cannot hang the test.
  for(const request of h.requests.slice(before))request.result.resolve([{id:'stale-created-A'}]);
  assert.equal((await creating).id,'created-A');
  assert.equal(h.requests.length,before,'late create must not issue a GET in the replacement context');
  assert.equal(h.store.list[0].id,returnToA?'loop-A':'loop-B');
});
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
function persistentGoalPage() {
  const form=goal();const parent={creating:true};let closes=0;
  // Execute the current parent's callback, rather than inventing a parent that
  // unmounts automatically on workspace change (the real page persists).
  const page=readFileSync(new URL('../src/modules/loops/LoopsPage.svelte',import.meta.url),'utf8');
  const expression=page.match(/<GoalDefineForm\s+oncancel=\{([^}]+)\}/)?.[1];
  assert.ok(expression,'parent form cancellation callback is present');
  const cancel=runInNewContext(`(${expression})`,parent);
  form.ctx.oncancel=()=>{closes++;cancel();};
  async function select(id:string) {
    if(parent.creating&&!await form.mayLeave(''))return false;
    form.ctx.ws.currentId=id;
    if(!parent.creating)form.dispose();
    return true;
  }
  return {...form,parent,select,closes:()=>closes};
}
test('goal workspace Keep retains A; Discard closes the persistent parent and never resurrects A',async()=>{
  const h=persistentGoalPage();h.ctx.seed='A goal';h.ctx.repoPath='/A';h.ctx.maxMinutes=120;
  h.ctx.draft={definition:{acceptance_criteria:[{id:'c1',text:'A criterion',verify:'A proof'}]}};
  assert.equal(await h.select('ws-B'),false);assert.equal(h.ctx.ws.currentId,'ws-A');
  assert.equal(h.parent.creating,true);assert.equal(h.closes(),0);assert.equal(h.ctx.maxMinutes,120);assert.equal(h.ctx.draft.definition.acceptance_criteria[0].text,'A criterion');
  h.setConfirm(true);assert.equal(await h.select('ws-B'),true);
  assert.equal(h.parent.creating,false,'accepted leave must settle parent creation state');assert.equal(h.closes(),1);
  await h.select('ws-A');assert.equal(h.parent.creating,false);assert.equal(h.closes(),1);assert.equal(h.prompts(),2);
});
test('goal clean workspace leave settles the parent without prompting',async()=>{
  const h=persistentGoalPage();await h.select('ws-B');assert.equal(h.parent.creating,false);assert.equal(h.closes(),1);assert.equal(h.prompts(),0);
});
test('goal local Cancel calls the real parent once',async()=>{
  const h=persistentGoalPage();h.ctx.seed='draft';h.setConfirm(true);await h.ctx.cancel();assert.equal(h.parent.creating,false);assert.equal(h.closes(),1);assert.equal(h.prompts(),1);
});
test('goal discarded pending launch uses actual store without replacing B or redirecting its parent',async()=>{
  const h=persistentGoalPage(), l=loopStore();h.ctx.loops=l.store;
  await l.load('ws-A');h.ctx.seed='A goal';h.ctx.repoPath='/A';h.ctx.name='A goal';
  h.ctx.draft={definition:{acceptance_criteria:[{id:'c1',text:'A',verify:'proof'}]},suggested_config:{executors:[]}};
  const launch=h.ctx.launch();h.setConfirm(true);await h.select('ws-B');await l.load('ws-B');
  const before=l.requests.length;l.created.resolve({id:'created-A'});await flush();
  for(const request of l.requests.slice(before))request.result.resolve([{id:'stale-A'}]);await launch;
  assert.equal(h.parent.creating,false);assert.equal(h.created.length,0);assert.equal(l.store.list[0].id,'loop-ws-B');assert.equal(l.requests.length,before);
});
