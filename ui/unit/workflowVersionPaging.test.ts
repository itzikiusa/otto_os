import {test} from 'node:test';
import assert from 'node:assert/strict';
import {deferred,loadSource} from './sourceHarness.ts';
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

test('workflow version wrapper requests bounded summaries and an exclusive cursor, leaving full detail separate',async()=>{
  const h=history();await h.wrapper.listWorkflowVersions('wf',{limit:50,beforeVersion:100});
  const url=new URL(h.paths[0],'http://fixture');
  assert.equal(url.searchParams.get('summary'),'true');assert.equal(url.searchParams.get('limit'),'50');assert.equal(url.searchParams.get('before_version'),'100');
  const detail=await h.wrapper.getWorkflowVersion('wf',1);assert.equal(detail.instructions,'full instructions');assert.equal(detail.graph.nodes[0].params.prompt,'full body');
  assert.equal(h.paths.at(-1),'/workflows/wf/versions/1');
});

test('workflow version drawer keeps one bounded window and can page to the oldest version across a new save',async()=>{
  const h=history();
  const ctx=componentFunctions(new URL('../src/modules/workflows/WorkflowsPage.svelte',import.meta.url),['loadVersions'],{
    current:{id:'wf'},destroyed:false,viewGeneration:0,versionsGeneration:0,versions:[],versionsLoading:false,versionsError:null,
    versionsBefore:undefined,versionsHasMore:false,VERSION_PAGE_SIZE:50,
    listWorkflowVersions:h.wrapper.listWorkflowVersions,loadErrorText:String,
  });
  await ctx.loadVersions();
  assert.ok(ctx.versions.length<=50,'drawer handler must request one 50-row window');
  assert.equal(ctx.versions[0].version,123);
  const seen:number[]=ctx.versions.map((v:any)=>v.version);h.addVersion();
  for(let n=0;n<4&&ctx.versions.length===50;n++) {
    const before=ctx.versions.at(-1).version;
    await ctx.loadVersions(before);
    assert.ok(ctx.versions.length<=50,'older pages replace the handler’s current window rather than accumulating rows');
    assert.ok(ctx.versions.every((v:any)=>v.version<before),'older page is exclusive');
    seen.push(...ctx.versions.map((v:any)=>v.version));
  }
  assert.equal(seen.length,123);assert.equal(new Set(seen).size,123);assert.equal(seen.at(-1),1);
  assert.ok(!seen.includes(124),'an intervening newest save does not shift the older cursor');
});


test('version drawer rejects stale failure/finally and retries the exact failed older page',async()=>{
  const requests:{id:string;options:any;result:ReturnType<typeof deferred<any>>}[]=[];
  const ctx=componentFunctions(new URL('../src/modules/workflows/WorkflowsPage.svelte',import.meta.url),['loadVersions'],{
    current:{id:'A'},destroyed:false,viewGeneration:0,versionsGeneration:0,versions:[],versionsLoading:false,versionsError:null,
    versionsBefore:undefined,versionsHasMore:false,VERSION_PAGE_SIZE:50,loadErrorText:String,
    listWorkflowVersions:(id:string,options:any)=>{const result=deferred<any>();requests.push({id,options,result});return result.promise;},
  });
  const old=ctx.loadVersions(80);ctx.current={id:'B'};ctx.viewGeneration++;
  const current=ctx.loadVersions(30);requests[0].result.reject(Error('stale A'));await old;
  assert.equal(ctx.versionsLoading,true);assert.equal(ctx.versionsError,null);assert.equal(ctx.versionsBefore,30);
  requests[1].result.reject(Error('B offline'));await current;assert.match(String(ctx.versionsError),/B offline/);
  const retry=ctx.loadVersions(ctx.versionsBefore);assert.equal(requests[2].id,'B');assert.equal(requests[2].options.beforeVersion,30);
  requests[2].result.resolve([{id:'b29',workflow_id:'B',version:29}]);await retry;
  assert.equal(ctx.versions[0].version,29);assert.equal(ctx.versionsError,null);assert.equal(ctx.versionsLoading,false);
});
