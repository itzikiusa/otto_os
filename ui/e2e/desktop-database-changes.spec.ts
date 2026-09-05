import { test, expect } from '@playwright/test';
import { apiCtx, seedWorkspace } from './seed';
import type { DatabaseChange } from '../src/lib/api/types';

// Browser contract checks. Resource authorization and real MySQL/Postgres
// execution are covered by Rust tests; these fixtures never send SQL.
test('database change draft binds validation to selected executor before submission', async ({page}) => {
  const {ctx,base}=await apiCtx();const workspaceId=await seedWorkspace(ctx,base);
  const meResponse=await ctx.get(`${base}/api/v1/auth/me`);const meBody=await meResponse.json();const me=meBody.user ?? meBody.effective_user ?? meBody;
  const response=await ctx.post(`${base}/api/v1/workspaces/${workspaceId}/connections`,{data:{name:'Migration fixture',kind:'mysql',params:{host:'example.invalid',user:'fixture',database:'shop'},secret:null,environment:'prod',read_only:false}});
  expect(response.ok()).toBeTruthy();const connection=(await response.json()).id;await ctx.dispose();
  let change:DatabaseChange|null=null;let validated=false;let submitted=false;
  await page.route(`**/api/v1/access/connection/${connection}/capabilities**`,route=>route.fulfill({json:{kind:'connection',resource_id:connection,user_id:me.id,mode:'enforced',child:null,operations:Object.fromEntries(['discover','db_browse','db_query','change_submit','change_approve','change_execute'].map(op=>[op,{allowed:true,reason:'fixture',matched_rule_ids:[],mode:'enforced'}]))}}));
  await page.route(`**/api/v1/connections/${connection}/db/**`,route=>{
    const path=new URL(route.request().url()).pathname;
    if(path.endsWith('/capabilities'))return route.fulfill({json:{engine:'mysql',supports_query:true,supports_schema:true,supports_explain:true,supports_cancel:false,supports_transactions:true}});
    if(path.endsWith('/test'))return route.fulfill({json:{ok:true,latency_ms:1,server_version:'fixture'}});
    return route.fulfill({json:[]});
  });
  await page.route('**/api/v1/database-changes**',async route=>{
    const request=route.request();const path=new URL(request.url()).pathname;
    if(path.endsWith('/executors'))return route.fulfill({json:[{id:me.id,display_name:'Fixture executor',username:'root'}]});
    if(path.endsWith('/validate')){expect(request.postDataJSON()).toMatchObject({revision:1,executor_id:me.id});validated=true;change={...change!,status:'validated',executor_id:me.id,content_hash:'immutable-fixture-hash'};return route.fulfill({json:change});}
    if(path.endsWith('/submit')){expect(validated).toBeTruthy();submitted=true;change={...change!,status:'awaiting_review'};return route.fulfill({json:change});}
    if(path.endsWith('/change-1'))return route.fulfill({json:{change,attempts:[],history:[]}});
    if(request.method()==='POST'){
      const input=request.postDataJSON();expect(input.targets).toEqual([{connection_id:connection,node:'shop'}]);expect(input.script).toBe('ALTER TABLE orders ADD reviewed_column INT;');
      change={...input,id:'change-1',author_id:me.id,real_author_id:me.id,revision:1,status:'draft',content_hash:'',executor_id:null,validation:{},approved_by:null,approved_real_by:null,approval_hash:null,cancellation_requested:false,created_at:new Date().toISOString(),updated_at:new Date().toISOString()};return route.fulfill({json:change});
    }
    return route.fulfill({json:change?[change]:[]});
  });
  await page.addInitScript(id=>{localStorage.setItem('otto_workspace',id);localStorage.setItem('otto_connhub_filter','all');},workspaceId);
  await page.goto('/#/connections');
  await page.getByRole('button',{name:/Migration fixture.*mysql/}).first().click();
  await page.getByRole('button',{name:'Changes',exact:true}).click();
  const panel=page.getByTestId('database-changes');
  await panel.getByRole('button',{name:'New change',exact:true}).click();
  await panel.getByLabel('Title',{exact:true}).fill('Add reviewed column');
  await panel.getByLabel('Database 1',{exact:true}).fill('shop');
  await panel.getByLabel('SQL script',{exact:true}).fill('ALTER TABLE orders ADD reviewed_column INT;');
  await panel.getByRole('button',{name:'Save draft',exact:true}).click();
  await expect(panel.getByRole('heading',{name:'Add reviewed column',exact:true})).toBeVisible();
  await panel.getByLabel('Executor',{exact:true}).selectOption(me.id);
  await panel.getByRole('button',{name:'Validate for executor',exact:true}).click();
  await panel.getByRole('button',{name:'Submit for review',exact:true}).click();
  await expect(panel.getByRole('button',{name:'Approve revision',exact:true})).toBeDisabled();
  await expect(panel.getByText('An independent reviewer must approve this change.',{exact:true})).toBeVisible();
  expect(submitted).toBeTruthy();
});
