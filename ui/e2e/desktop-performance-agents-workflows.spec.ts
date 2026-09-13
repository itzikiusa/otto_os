import {test, expect, type APIRequestContext} from '@playwright/test';
import {apiCtx, seedWorkspace} from './seed';

// Route-boundary fixtures drive the production mounted components and stores.
// No provider or workflow is launched. Root runs this on the isolated E2E slot.
let ctx: APIRequestContext, base='', workspace='';
const now='2026-09-13T10:00:00Z';
function entry(id:string) {return {session_id:id,provider:'claude',title:`Fixture ${id}`,first_prompt:null,cwd:'/tmp/otto-fixture',repo_name:null,started_at:now,last_active_at:now,turns:1,status:'exited',transcript_path:`/tmp/${id}.jsonl`,resumable:false};}
function transcript(id:string) {return {session_id:id,provider:'claude',title:id,cwd:'/tmp',model:null,cursor:'0',has_earlier:false,turns:[{id,role:'assistant',ts:now,blocks:[{kind:'text',md:`Body ${id}`}],duration_ms:null,model:null,system:[],reasoning_steps:0}],stats:{turns:1,input_tokens:0,output_tokens:0,cache_read_tokens:0,cache_creation_tokens:0,cost_usd:null},subagents:[],unavailable_reason:null};}
test.beforeEach(async({page},info)=>{
  test.skip(info.project.name!=='desktop-browser','desktop fixture');
  ({ctx,base}=await apiCtx());workspace=await seedWorkspace(ctx,base);
  await page.addInitScript(id=>{localStorage.setItem('otto_workspace',id);localStorage.setItem('otto_firstrun_dismissed','1');},workspace);
});
test.afterEach(async()=>{await ctx?.dispose();});

test('empty History continuation remains actionable and mounted visibility recovery ignores closed readers',async({page})=>{
  const reads:string[]=[],touches:string[]=[];
  await page.route('**/api/v1/workspaces/*/history/page?*',route=>route.fulfill({json:{entries:new URL(route.request().url()).searchParams.has('cursor')?[entry('reader-a'),entry('reader-b')]:[],next_cursor:new URL(route.request().url()).searchParams.has('cursor')?null:'next-window'}}));
  await page.route('**/api/v1/sessions/reader-*/transcript**',route=>{
    const id=new URL(route.request().url()).pathname.split('/')[4];
    if(route.request().method()==='POST'){touches.push(id);return route.fulfill({status:204});}
    reads.push(id);return route.fulfill({json:transcript(id)});
  });
  await page.goto('/#/history');
  await expect(page.getByText('No matches in this part of history. Load more to keep looking.')).toBeVisible();
  await page.getByRole('button',{name:'Load more',exact:true}).click();
  await page.locator('[data-session-id="reader-a"]').click();
  await expect(page.getByText('Body reader-a',{exact:true})).toBeVisible();
  await page.locator('[data-session-id="reader-b"]').click();
  await expect(page.getByText('Body reader-b',{exact:true})).toBeVisible();
  await expect.poll(()=>touches.length).toBe(2);reads.length=0;touches.length=0;
  await page.evaluate(()=>{Object.defineProperty(document,'hidden',{configurable:true,value:true});document.dispatchEvent(new Event('visibilitychange'));});
  await page.evaluate(()=>{Object.defineProperty(document,'hidden',{configurable:true,value:false});document.dispatchEvent(new Event('visibilitychange'));});
  await expect.poll(()=>reads.length).toBe(1);await expect.poll(()=>touches.length).toBe(1);
  expect(reads).toEqual(['reader-b']);expect(touches).toEqual(['reader-b']);
  // Same mounted source must acquire a new lease when its identity is revoked.
  reads.length=0;touches.length=0;
  await page.evaluate(async()=>{
    const path='/src/lib/stores/transcript.svelte.ts';const {transcript}=await import(/* @vite-ignore */path);
    transcript.setIdentity('isolated-browser-new-owner');
  });
  await expect.poll(()=>reads.length).toBe(1);await expect.poll(()=>touches.length).toBe(1);
  expect(reads).toEqual(['reader-b']);
});

test('workflow summary polls leave closed bodies unloaded and expansion fetches exact version once',async({page})=>{
  const workflow={id:'wf-performance',workspace_id:workspace,name:'Lazy workflow fixture',description:'',icon:'',graph:{nodes:[{id:'step',kind:'log',name:'Lazy step',x:0,y:0,params:{}}],edges:[]},created_at:now,updated_at:now,created_by:'fixture',version:1};
  const node={node_id:'step',status:'success',started_at:now,duration_ms:1,attempts:1,sessions:[],activity:null,error:null,logs:[],output:null,log_count:1,has_output:true,detail_version:'body-v1'};
  const run={id:'run-performance',workflow_id:workflow.id,workspace_id:workspace,status:'running',nodes:[node],checkpoints:[],input:null,error:null,started_at:now,finished_at:now,rev:1,waiting_approval:false,workflow_version:1,resume_attempts:0,summary:true,checkpoint_rev:0,checkpoint_generation:0,checkpoint_count:0,checkpoint_done:0};
  let details=0,fullReads=0,conditional=0;
  await page.route('**/api/v1/workspaces/*/workflows',route=>route.fulfill({json:[workflow]}));
  await page.route('**/api/v1/workflows/wf-performance',route=>route.fulfill({json:workflow}));
  await page.route('**/api/v1/workflows/wf-performance/runs?summary=true',route=>route.fulfill({json:[{id:run.id,workflow_id:workflow.id,status:run.status,started_at:now,rev:1}]}));
  await page.route('**/api/v1/workflow-runs/run-performance**',route=>{
    const url=new URL(route.request().url());
    if(url.pathname.endsWith('/progress')){if(url.searchParams.has('after_rev'))conditional++;return route.fulfill({json:url.searchParams.has('after_rev')?{changed:false,rev:1}:{changed:true,rev:1,run}});}
    if(url.pathname.endsWith('/nodes/step')){details++;return route.fulfill({json:{rev:1,detail_version:'body-v1',body:{...node,logs:['Exact lazy log'],output:{sentinel:'Exact lazy output'}}}});}
    if(url.pathname.endsWith('/checkpoints'))return route.fulfill({json:{items:[],generation:0,checkpoint_rev:0,next_cursor:null}});
    fullReads++;return route.fulfill({json:run});
  });
  await page.goto('/#/workflows');
  await page.getByTestId(`wf-row-${workflow.id}`).locator('.row-main').click();
  await page.getByRole('button',{name:'Runs',exact:true}).click();await page.getByTestId('run-item').first().click();
  await expect(page.locator('.run-detail details.step')).toBeVisible();
  await expect.poll(()=>conditional).toBeGreaterThan(0);
  expect(details).toBe(0);expect(fullReads).toBe(0);
  await page.locator('.run-detail details.step > summary').click();
  await expect(page.getByText('Exact lazy log',{exact:true})).toBeVisible();expect(details).toBe(1);
  await page.locator('.run-detail details.step > summary').click();await page.locator('.run-detail details.step > summary').click();
  await expect(page.getByText('Exact lazy log',{exact:true})).toBeVisible();expect(details).toBe(1);expect(fullReads).toBe(0);
});
