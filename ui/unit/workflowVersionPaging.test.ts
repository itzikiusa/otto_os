import {test} from 'node:test';
import assert from 'node:assert/strict';
import {loadSource} from './sourceHarness.ts';
import {componentFunctions} from './componentFunctions.ts';

function history() {
  const paths:string[]=[];
  let latest=123;
  const rows=()=>Array.from({length:latest},(_,i)=>({id:`v${latest-i}`,workflow_id:'wf',version:latest-i,note:'saved',created_at:'2026-10-05T00:00:00Z'}));
  const api={get:async(path:string)=>{
    paths.push(path);const url=new URL(path,'http://fixture');
    const before=Number(url.searchParams.get('before_version')??Infinity);
    const limit=Number(url.searchParams.get('limit')??100);
    if(url.pathname.endsWith('/versions'))return rows().filter(row=>row.version<before).slice(0,Math.min(100,Math.max(1,limit)));
    return {...rows().find(row=>url.pathname.endsWith(`/${row.version}`)),graph:{nodes:[{id:'old',params:{prompt:'full body'}}],edges:[]},instructions:'full instructions'};
  },post:async()=>({id:'wf'})};
  const wrapper=loadSource(new URL('../src/lib/api/workflows.ts',import.meta.url),{'./client':{api}});
  return {paths,wrapper,addVersion:()=>latest++};
}

test('workflow version wrapper requests bounded summaries and an exclusive cursor',async()=>{
  const h=history();await h.wrapper.listWorkflowVersions('wf',{limit:50,beforeVersion:100});
  const url=new URL(h.paths[0],'http://fixture');
  assert.equal(url.searchParams.get('summary'),'true');assert.equal(url.searchParams.get('limit'),'50');assert.equal(url.searchParams.get('before_version'),'100');
  assert.equal(h.wrapper.getWorkflowVersion,undefined,'the unused single-version getter is gone; restore reads the version server-side');
});

test('the Versions panel pages to the oldest version, a save made meanwhile never shifts the cursor',async()=>{
  const h=history();
  const ctx=componentFunctions(new URL('../src/modules/workflows/editor/EditorPage.svelte',import.meta.url),['openVersions'],{
    workflowId:'wf',panel:null,versions:null,versionsError:null,versionsMore:false,VERSION_PAGE:30,
    listWorkflowVersions:h.wrapper.listWorkflowVersions,loadErrorText:String,
  });
  await ctx.openVersions();
  assert.equal(ctx.panel,'versions');assert.equal(ctx.versions.length,30);assert.equal(ctx.versions[0].version,123);
  h.addVersion();
  for(let n=0;n<10&&ctx.versionsMore;n++) await ctx.openVersions(ctx.versions.at(-1).version);
  const seen=ctx.versions.map((v:any)=>v.version);
  assert.equal(seen.length,123);assert.equal(new Set(seen).size,123);assert.equal(seen.at(-1),1);
  assert.ok(!seen.includes(124),'a save made meanwhile does not shift the older cursor');
  assert.equal(ctx.versionsMore,false);
  for(const p of h.paths.filter((x)=>x.includes('/versions'))) assert.ok(new URL(p,'http://f').searchParams.get('limit')==='30');
});
