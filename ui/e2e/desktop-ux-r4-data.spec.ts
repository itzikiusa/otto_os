import { test, expect, type Page } from '@playwright/test';
import { apiCtx, seedWorkspace } from './seed';
import { openPage, openApiEditor, expectNoHorizontalOverflow, expectFullyInViewport } from './helpers';
import { mkdirSync } from 'node:fs';
test.use({serviceWorkers:'block'});
const shots='/tmp/otto-ux-r4-data-screenshots';
// Connection tabs can restore a previous synthetic broker during navigation.
test.beforeEach(async ({page}) => {
  await page.route('**/brokers/clusters/*/**', r => r.fulfill({json:[]}));
});
async function workspace(page:Page) {
  const {ctx,base}=await apiCtx(); const id=await seedWorkspace(ctx,base);
  await page.addInitScript(id=>{localStorage.setItem('otto_workspace',id);localStorage.setItem('otto_firstrun_dismissed','1');},id);
  return {ctx,base,id};
}
async function database(page:Page) {
  const {ctx,base,id}=await workspace(page);
  const rows=[];
  for(const name of ['Primary','Alternate']) {
    const r=await ctx.post(`${base}/api/v1/workspaces/${id}/connections`,{data:{name:`R4 ${name} ${id}`,kind:'mysql',params:{host:'fixture.invalid',port:3306}}});
    expect(r.ok()).toBeTruthy(); rows.push(await r.json());
  }
  await ctx.dispose();
  await page.route('**/db/capabilities',r=>r.fulfill({json:{engine:'mysql',query_language:'sql',can_write:true,explain:true}}));
  await page.route('**/db/schema**',r=>r.fulfill({json:[]}));
  await openPage(page,'database');
  await page.locator('.conn-list .conn-name',{hasText:rows[0].name}).click();
  await expect(page.locator('.qe-edit .cm-content')).toBeVisible();
  return rows;
}

async function showConnections(page:Page) {
  const tab=page.getByRole('tablist',{name:'Sidebar view'}).getByRole('tab',{name:'Connections',exact:true});
  if(await tab.isVisible()) await tab.click();
  else {
    const accordion=page.getByRole('button',{name:/^Connections \d+/});
    if(await accordion.getAttribute('aria-expanded')==='false') await accordion.click();
  }
}

test('query save ignores repeated keyboard submission while a response is pending',async({page})=>{
  let release!:()=>void;const held=new Promise<void>(r=>release=r);let posts=0;
  await page.route('**/saved-queries',async r=>{
    if(r.request().method()!=='POST')return r.fulfill({json:[]});
    posts++;await held;return r.fulfill({json:{...r.request().postDataJSON(),id:'saved',created_at:'2026-09-26'}});
  });
  await database(page);await page.locator('.qe-edit .cm-content').fill('SELECT 41');
  await page.getByRole('button',{name:'Save',exact:true}).click();
  const name=page.getByPlaceholder('Query name');await name.fill('Only once');
  await name.press('Enter');await expect.poll(()=>posts).toBe(1);await name.press('Enter');
  await expect(page.locator('.save-bar').getByRole('button',{name:/Saving|Save/,exact:false}).first()).toBeDisabled();
  expect(posts).toBe(1);release();await expect(page.locator('.save-bar')).toBeHidden();
});

test('saved query response stays associated with its original connection after switching away and back',async({page})=>{
  let release!:()=>void;const held=new Promise<void>(r=>release=r);let sent=false;
  await page.route('**/saved-queries',async r=>{
    if(r.request().method()!=='POST')return r.fulfill({json:[]});
    sent=true;await held;return r.fulfill({json:{...r.request().postDataJSON(),id:'saved-original',created_at:'2026-09-26'}});
  });
  const rows=await database(page);await page.locator('.qe-edit .cm-content').fill('SELECT primary_draft');
  await page.getByRole('button',{name:'Save',exact:true}).click();await page.getByPlaceholder('Query name').fill('Primary saved query');
  await page.locator('.save-bar').getByRole('button',{name:'Save',exact:true}).click();await expect.poll(()=>sent).toBe(true);
  await showConnections(page);
  await page.locator('.conn-list .conn-name',{hasText:rows[1].name}).click();
  await page.locator('.qe-edit .cm-content').fill('SELECT alternate_draft');
  const done=page.waitForResponse(r=>r.url().endsWith('/saved-queries')&&r.request().method()==='POST');release();await done;
  await expect(page.locator('.qe-edit .cm-content')).toHaveText('SELECT alternate_draft');
  await showConnections(page);
  await page.locator('.conn-list .conn-name',{hasText:rows[0].name}).click();
  await expect(page.locator('.qe-edit .cm-content')).toHaveText('SELECT primary_draft');
  await expect(page.getByRole('tablist',{name:'Query tabs'}).getByRole('tab',{selected:true})).toContainText('Primary saved query');
  await expect(page.getByRole('button',{name:'Update',exact:true})).toBeVisible();
});

for(const variant of [
  {name:'native-light',theme:'native',scheme:'light',direction:'ltr',width:1440,height:900},
  {name:'native-dark',theme:'native',scheme:'dark',direction:'ltr',width:1440,height:900},
  {name:'warm-light',theme:'warm',scheme:'light',direction:'ltr',width:1440,height:900},
  {name:'warm-dark',theme:'warm',scheme:'dark',direction:'ltr',width:1440,height:900},
  {name:'pro-dark',theme:'pro-dark',scheme:'dark',direction:'ltr',width:1440,height:900},
  {name:'phone',theme:'native',scheme:'light',direction:'ltr',width:375,height:812},
  {name:'tablet-rtl',theme:'warm',scheme:'dark',direction:'rtl',width:834,height:1112},
])test(`embedded network profile form uses its sheet scroll and keyboard saves eight endpoints ${variant.name}`,async({page})=>{
  await page.setViewportSize(variant);
  await page.addInitScript(v=>{localStorage.setItem('otto_theme',v.theme);localStorage.setItem('otto_scheme',v.scheme);localStorage.setItem('otto_direction',v.direction);},variant);
  const {ctx,id}=await workspace(page);await ctx.dispose();
  const profile={id:'profile',name:'Reporting services',workspace_id:id,version:1,ssh_connection_id:'ssh',archived:false,endpoints:[{name:'Reporting database',remote_host:'reporting-db.fixture.internal',remote_port:5432,host_env:'PGHOST',port_env:'PGPORT'}]};
  let saved:any;
  await page.route('**/network-profiles',r=>r.fulfill({json:[profile]}));
  await page.route('**/network-profiles/profile',r=>{saved=r.request().postDataJSON();return r.fulfill({json:{...profile,...saved,version:2}});});
  await page.route('**/connections',r=>r.fulfill({json:[{id:'ssh',name:'Reporting bastion',kind:'ssh'}]}));
  await openPage(page,'agents');await page.keyboard.press('Meta+t');
  const sheet=page.getByRole('dialog',{name:'New session',exact:true});await expect(sheet).toBeVisible();
  await sheet.getByRole('button',{name:'Manage network profiles'}).click();await sheet.getByRole('button',{name:'Edit Reporting services'}).click();
  mkdirSync(shots,{recursive:true});await sheet.locator('.manager').scrollIntoViewIfNeeded();
  await page.screenshot({animations:'disabled',path:`${shots}/embedded-${variant.name}.png`});
  expect(await sheet.locator('.manager').evaluate(el=>el.scrollHeight-el.clientHeight)).toBeLessThanOrEqual(1);
  if(variant.name==='phone'||variant.name==='tablet-rtl') {
    for(let i=2;i<=8;i++) {
      await sheet.getByRole('button',{name:'Add endpoint',exact:true}).focus();await page.keyboard.press('Enter');
      await expect(sheet.getByLabel(`Endpoint ${i} name`,{exact:true})).toBeFocused();
      await page.keyboard.type(`Service ${i}`);await page.keyboard.press('Tab');await page.keyboard.type(`service-${i}.fixture.internal`);
    }
    await expect(sheet.getByRole('button',{name:'Add endpoint',exact:true})).toBeDisabled();
    await sheet.getByRole('button',{name:'Save network profile',exact:true}).focus();await page.keyboard.press('Enter');
    await expect.poll(()=>saved?.endpoints?.length).toBe(8);
    await expect(sheet.getByRole('button',{name:'Remove endpoint 8',exact:true})).toBeEnabled();
    await sheet.getByRole('button',{name:'Remove endpoint 8',exact:true}).focus();await page.keyboard.press('Enter');
    await expect(sheet.getByLabel('Endpoint 7 name',{exact:true})).toBeFocused();
    await expectFullyInViewport(page,sheet.getByLabel('Endpoint 7 name',{exact:true}));
  }
  await expectNoHorizontalOverflow(page);
});

async function broker(page:Page) {
  const {ctx,base,id}=await workspace(page);
  const r=await ctx.post(`${base}/api/v1/workspaces/${id}/brokers/clusters`,{data:{name:`R4 broker ${id}`,bootstrap_servers:'fixture.invalid:9092',security_protocol:'plaintext',environment:'dev',read_only:false,schema_registry_url:'https://fixture.invalid'}});
  expect(r.ok()).toBeTruthy();const cluster=await r.json();await ctx.dispose();
  await page.route('**/brokers/clusters/*/overview',r=>r.fulfill({json:{brokers:[],topic_count:1,internal_topic_count:0,partition_count:1,consumer_group_count:0,controller_id:0}}));
  await page.route('**/brokers/clusters/*/test',r=>r.fulfill({json:{ok:true,latency_ms:1}}));
  await page.route('**/brokers/clusters/*/metrics',r=>r.fulfill({json:{messages_per_sec:0,throughput:[]}}));
  await page.route('**/topics',r=>r.fulfill({json:[{name:'customer-events',internal:false,partition_count:1,replication_factor:1}]}));
  await page.route('**/topics/customer-events/stats',r=>r.fulfill({json:{message_count:1,cleanup_policy:'delete'}}));
  await page.route('**/topics/customer-events',r=>r.fulfill({json:{name:'customer-events',message_count:1,partitions:[{id:0,leader:1,replicas:[1],isr:[1],low:0,high:1,message_count:1}],configs:[]}}));
  await openPage(page,'brokers');await page.locator('.cluster .cn',{hasText:cluster.name}).click();
}

test('Kafka consume validates selectors before reading and produce preserves failed drafts',async({page,isMobile})=>{
  let read:any;let produced:any;let fail=true;
  await page.route('**/topics/customer-events/consume',r=>{read=r.request().postDataJSON();return r.fulfill({json:{messages:[{partition:0,offset:0,timestamp_ms:1790377200000,key:{format:'utf8',text:'customer-1'},value:{format:'json',text:'{"name":"Synthetic customer"}'},headers:[],size_bytes:28},{partition:0,offset:1,timestamp_ms:1790377200001,key:{format:'utf8',text:'customer-2'},value:{format:'json',text:'{"name":"Second synthetic customer"}'},headers:[],size_bytes:35}],partitions:[{partition:0,low:0,high:2}],truncated:false}});});
  await page.route('**/topics/customer-events/produce',r=>{produced=r.request().postDataJSON();return fail?r.fulfill({status:503,json:{code:'upstream',message:'Synthetic broker unavailable'}}):r.fulfill({json:{partition:0,offset:1}});});
  await broker(page);await page.locator('.tabs button',{hasText:'Topics'}).click();await page.getByText('customer-events',{exact:true}).click();
  const peek=page.getByRole('button',{name:'Peek',exact:true});
  await page.getByLabel('Start position').selectOption('timestamp');await expect(peek).toBeDisabled();
  await page.getByLabel('Start time').fill('2026-09-26T10:00');await expect(peek).toBeEnabled();
  await page.getByLabel('Start position').selectOption('offset');
  for(const value of ['','-1','1.5']) {await page.getByLabel('Start offset').fill(value);await expect(peek).toBeDisabled();}
  await page.getByLabel('Start offset').fill('0');
  for(const value of ['','0','5001','1.5']) {await page.getByLabel('Max messages').fill(value);await expect(peek).toBeDisabled();}
  await page.getByLabel('Max messages').fill('50');await peek.click();await expect(page.locator('.msg-list')).toContainText('customer-1');
  expect(read.start).toEqual({type:'offset',offset:0});
  const inspect=page.getByRole('button',{name:'Inspect partition 0 offset 0',exact:true});
  await expect(inspect).toBeVisible();
  const rows=page.locator('.msg-list tbody tr');
  await rows.nth(1).locator('.key').click();await expect(page.locator('.msg-detail')).toContainText('Second synthetic customer');
  await expect(page.getByRole('button',{name:'Inspect partition 0 offset 1',exact:true})).toHaveAttribute('aria-pressed','true');
  await rows.nth(0).locator('td').first().click();await expect(page.locator('.msg-detail')).toContainText('Synthetic customer');
  await expect(inspect).toHaveAttribute('aria-pressed','true');
  await rows.nth(1).locator('.pos-bar-wrap').click();await expect(page.locator('.msg-detail')).toContainText('Second synthetic customer');
  await rows.nth(0).locator('td').first().click({button:'right'});
  await expect(page.getByRole('button',{name:'Inspect partition 0 offset 1',exact:true})).toHaveAttribute('aria-pressed','true');
  if (!isMobile) {
    const key=await rows.nth(0).locator('.key').boundingBox();expect(key).not.toBeNull();
    await page.mouse.move(key!.x+3,key!.y+key!.height/2);await page.mouse.down();
    await page.mouse.move(key!.x+key!.width-3,key!.y+key!.height/2,{steps:8});await page.mouse.up();
    expect(await page.evaluate(()=>window.getSelection()?.toString())).toContain('customer');
    await expect(page.getByRole('button',{name:'Inspect partition 0 offset 1',exact:true})).toHaveAttribute('aria-pressed','true');
    await page.evaluate(()=>window.getSelection()?.removeAllRanges());
  }
  await inspect.focus();await page.keyboard.press('Enter');await expect(page.locator('.msg-detail')).toContainText('Synthetic customer');
  await expect(inspect).toHaveAttribute('aria-pressed','true');
  const secondInspect=page.getByRole('button',{name:'Inspect partition 0 offset 1',exact:true});
  await secondInspect.focus();await page.keyboard.press('Space');await expect(page.locator('.msg-detail')).toContainText('Second synthetic customer');
  await expect(secondInspect).toHaveAttribute('aria-pressed','true');
  mkdirSync(shots,{recursive:true});await page.screenshot({animations:'disabled',path:`${shots}/kafka-consume.png`});
  await page.locator('.subtabs').getByRole('button',{name:'Produce',exact:true}).click();
  await page.getByLabel('Key (optional)',{exact:true}).fill('customer-2');await page.getByLabel('Value',{exact:true}).fill('{"name":"Second fixture"}');
  await page.getByRole('button',{name:'Produce message',exact:true}).click();await expect(page.getByText('Synthetic broker unavailable',{exact:true})).toBeVisible();
  await expect(page.getByLabel('Value',{exact:true})).toHaveValue('{"name":"Second fixture"}');
  fail=false;await page.getByRole('button',{name:'Produce message',exact:true}).click();await expect(page.getByLabel('Value',{exact:true})).toHaveValue('');
  expect(produced).toMatchObject({key:'customer-2',value:'{"name":"Second fixture"}',partition:null});
});

test('Kafka masked live tail keeps masking incremental reads and labels mixed results honestly',async({page})=>{
  const reads:any[]=[];
  await page.route('**/topics/customer-events/consume',r=>{
    const req=r.request().postDataJSON();reads.push(req);const offset=reads.length-1;
    return r.fulfill({json:{messages:[{partition:0,offset,timestamp_ms:1790377200000,key:{format:'utf8',text:`event-${offset}`},value:{format:'utf8',text:req.mask?'[MASKED]':'Synthetic unmasked value'},headers:[],size_bytes:16}],partitions:[{partition:0,low:0,high:offset+1}],truncated:false,masked:req.mask===true}});
  });
  await broker(page);await page.locator('.tabs button',{hasText:'Topics'}).click();await page.getByText('customer-events',{exact:true}).click();
  await page.clock.install();
  await page.getByRole('checkbox',{name:'Mask',exact:true}).check();await page.getByRole('checkbox',{name:'Live · 1m',exact:true}).check();
  await expect(page.locator('.msg-list tbody tr')).toHaveCount(1);expect(reads[0].mask).toBe(true);
  await expect(page.locator('.masked-badge')).toBeVisible();
  await page.clock.fastForward(60_000);await expect(page.locator('.msg-list tbody tr')).toHaveCount(2);
  expect(reads[1]).toMatchObject({partition:0,start:{type:'offset',offset:1},mask:true});await expect(page.locator('.masked-badge')).toBeVisible();
  await page.getByRole('button',{name:'Inspect partition 0 offset 1',exact:true}).click();await expect(page.locator('.msg-detail')).toContainText('[MASKED]');
  await page.getByRole('checkbox',{name:'Mask',exact:true}).uncheck();await page.clock.fastForward(60_000);await expect(page.locator('.msg-list tbody tr')).toHaveCount(3);
  expect(reads[2].mask).toBeUndefined();await expect(page.locator('.masked-badge')).toHaveCount(0);
  await page.getByRole('checkbox',{name:'Live · 1m',exact:true}).uncheck();
});

test('SFTP long transfer can be cancelled on phone without losing file navigation',async({page})=>{
  await page.setViewportSize({width:375,height:812});const {ctx,base,id}=await workspace(page);
  const r=await ctx.post(`${base}/api/v1/workspaces/${id}/connections`,{data:{name:`R4 files ${id}`,kind:'ssh',params:{host:'fixture.invalid'}}});expect(r.ok()).toBeTruthy();const conn=await r.json();await ctx.dispose();
  const transfer={id:'synthetic-transfer',direction:'download',remote_path:'/srv/customer-reporting/long-export-with-dated-partition-2026-09-26.json',status:'running',bytes:8192,total_bytes:10485760,elapsed_secs:31,error:null};
  await page.route('**/sftp/list**',r=>r.fulfill({json:{path:'/srv/customer-reporting',entries:[]}}));
  await page.route('**/sftp/transfers',r=>r.fulfill({json:[transfer]}));
  await page.route('**/sftp/transfers/synthetic-transfer/cancel',r=>{transfer.status='cancelled';return r.fulfill({json:transfer});});
  await openPage(page,'connections');await page.locator('.conn-row',{hasText:conn.name}).click({button:'right'});await page.getByRole('menuitem',{name:'Browse files (SFTP)',exact:true}).click();
  const transfers=page.getByLabel('File transfers',{exact:true});await expect(transfers).toContainText('Running');
  await expectFullyInViewport(page,transfers.getByRole('button',{name:'Cancel',exact:true}));await transfers.getByRole('button',{name:'Cancel',exact:true}).click();
  await expect(transfers).toContainText('Cancelled');await expect(page.locator('.sftp .crumbs')).toContainText('customer-reporting');
  await expect(transfers.locator('.ellipsis')).toHaveCSS('white-space','normal');
  await expectNoHorizontalOverflow(page);await page.screenshot({animations:'disabled',path:`${shots}/sftp-transfer-phone.png`});
});

for(const mode of ['sse','websocket'] as const)test(`API ${mode} connects receives and disconnects a synthetic relay`,async({page})=>{
  const {ctx}=await workspace(page);await ctx.dispose();
  let opened:any;let sent='';
  await page.routeWebSocket('**/ws/api-client/stream?*',socket=>{
    socket.onMessage(data=>{const msg=JSON.parse(String(data));if(msg.action==='open') {opened=msg;socket.send(JSON.stringify({type:'open',detail:'Fixture connected'}));socket.send(JSON.stringify(mode==='sse'?{type:'event',event:'customer.updated',data:'Synthetic customer update'}:{type:'message',dir:'in',data:'Synthetic welcome'}));}else if(msg.action==='send'){sent=msg.data;socket.send(JSON.stringify({type:'message',dir:'out',data:msg.data}));}});
  });
  await openApiEditor(page);await page.getByLabel('Request type').selectOption(mode);await page.getByLabel('Request URL',{exact:true}).fill(mode==='sse'?'https://fixture.invalid/events':'wss://fixture.invalid/messages');
  await page.getByRole('button',{name:'Connect',exact:true}).click();await expect(page.locator('.stream-log')).toContainText(mode==='sse'?'Synthetic customer update':'Synthetic welcome');expect(opened.kind).toBe(mode);
  if(mode==='websocket'){await page.getByLabel('Message to send').fill('Synthetic reply');await page.getByLabel('Message to send').press('Enter');await expect.poll(()=>sent).toBe('Synthetic reply');}
  mkdirSync(shots,{recursive:true});await page.screenshot({animations:'disabled',path:`${shots}/api-${mode}.png`});
  await page.getByRole('button',{name:'Disconnect',exact:true}).click();await expect(page.getByRole('button',{name:'Connect',exact:true})).toBeEnabled();
  if(mode==='websocket')await expect(page.getByLabel('Message to send')).toBeDisabled();
});

test('API gRPC reflection and invocation recovers after a failed call',async({page})=>{
  const {ctx}=await workspace(page);await ctx.dispose();let fail=true;let body:any;
  await page.route('**/api-client/grpc/reflect',r=>r.fulfill({json:{services:[{name:'customers.CustomerService',methods:[{name:'GetCustomer',full:'customers.CustomerService/GetCustomer',input_schema:'{"id":"fixture"}',server_streaming:false,client_streaming:false}]}]}}));
  await page.route('**/api-client/grpc/invoke',r=>{body=r.request().postDataJSON();return fail?r.fulfill({status:503,json:{code:'upstream',message:'Synthetic service unavailable'}}):r.fulfill({json:{status:200,status_text:'OK',headers:[],body:'{"name":"Synthetic customer"}',duration_ms:3,size_bytes:29,content_type:'application/json',truncated:false,too_large:false,trace:[]}});});
  await openApiEditor(page);await page.getByLabel('Request type').selectOption('grpc');await page.getByLabel('Request URL',{exact:true}).fill('https://fixture.invalid:443');await page.getByRole('button',{name:'Load from server',exact:true}).click();
  await expect(page.getByLabel('gRPC method')).toHaveValue('customers.CustomerService/GetCustomer');await page.getByRole('button',{name:'Invoke',exact:true}).click();await expect(page.getByText('Synthetic service unavailable',{exact:true})).toBeVisible();
  fail=false;await page.getByRole('button',{name:'Invoke',exact:true}).click();await expect(page.locator('.viewer')).toContainText('Synthetic customer');expect(body.method).toBe('customers.CustomerService/GetCustomer');expect(JSON.parse(body.body)).toEqual({id:'fixture'});
  await page.locator('.viewer').scrollIntoViewIfNeeded();
  await page.screenshot({animations:'disabled',path:`${shots}/api-grpc.png`});
});

test('Kafka schema tablet gives versions usable width and supports keyboard navigation',async({page})=>{
  await page.setViewportSize({width:834,height:1112});await page.addInitScript(()=>{localStorage.setItem('otto_theme','warm');localStorage.setItem('otto_scheme','dark');localStorage.setItem('otto_direction','rtl');});
  const schema=JSON.stringify({type:'record',name:'CustomerEvent',fields:[{name:'customer_identifier',type:'string'}]});
  await page.route('**/schema-registry/subjects',r=>r.fulfill({json:[{subject:'customer-reporting-events-value',id:1,version:2,schema_type:'AVRO',schema}]}));
  let fail=true;
  await page.route('**/schema-registry/subjects/*/versions',r=>fail?r.fulfill({status:503,json:{code:'upstream',message:'Synthetic versions unavailable'}}):r.fulfill({json:[{version:1,id:1,schema_type:'AVRO',schema:'{"type":"record","name":"CustomerEvent","fields":[]}'},{version:2,id:2,schema_type:'AVRO',schema}]}));
  let checked:any;await page.route('**/schema-registry/subjects/*/compatibility',r=>{checked=r.request().postDataJSON();return r.fulfill({json:{compatible:true,messages:[]}});});
  await broker(page);await page.locator('.tabs button',{hasText:'Schema Registry'}).click();
  await expect(page.locator('.schema .view .payload')).toContainText('customer_identifier');
  mkdirSync(shots,{recursive:true});await page.screenshot({animations:'disabled',path:`${shots}/schema-tablet.png`});
  const width=await page.locator('.schema .view').evaluate(el=>el.getBoundingClientRect().width);expect(width).toBeGreaterThan(300);
  const tabs=page.getByRole('tablist',{name:'Subject views'});await expect(tabs).toHaveAttribute('tabindex','-1');await tabs.getByRole('tab',{name:'Schema',exact:true}).focus();await page.keyboard.press('ArrowLeft');await expect(tabs.getByRole('tab',{name:'Versions & Compat'})).toBeFocused();
  await expect(page.getByText('Synthetic versions unavailable',{exact:true})).toBeVisible();fail=false;await page.locator('.svp').getByRole('button',{name:'Retry',exact:true}).click();
  await page.getByRole('button',{name:'Show diff v1 → v2'}).click();await expect(page.locator('.diff-section')).toContainText('customer_identifier');
  await page.locator('.compat-input').fill(schema);await page.getByRole('button',{name:'Check',exact:true}).click();await expect(page.locator('.compat-result')).toHaveText('Compatible');expect(checked).toEqual({schema});await expectNoHorizontalOverflow(page);
});

for(const engine of ['redis','mongodb'] as const)test(`${engine} result editing stages the native command for review without external writes`,async({page})=>{
  const {ctx,base,id}=await workspace(page);const r=await ctx.post(`${base}/api/v1/workspaces/${id}/connections`,{data:{name:`R4 ${engine} ${id}`,kind:engine,params:{host:'fixture.invalid',port:engine==='redis'?6379:27017},read_only:false,environment:'dev'}});expect(r.ok()).toBeTruthy();const conn=await r.json();await ctx.dispose();
  await page.route('**/db/capabilities',r=>r.fulfill({json:{engine,query_language:engine==='redis'?'redis':'mongo',can_write:true,sql:false}}));await page.route('**/db/schema**',r=>r.fulfill({json:[]}));
  const rows=engine==='redis'?{columns:[{name:'value',type_hint:'String'}],rows:[['Original value']]}:{columns:[{name:'_id',type_hint:'ObjectId'},{name:'status',type_hint:'String'},{name:'items',type_hint:'Array'}],rows:[[{ $oid:'507f1f77bcf86cd799439011'},'pending',[{qty:1}]]]};
  let queries=0;await page.route('**/db/query',r=>{queries++;expect(r.request().postDataJSON().statement).toBe(engine==='redis'?'GET synthetic:customer':'db.synthetic_customers.find({})');return r.fulfill({json:{...rows,stats:{duration_ms:2,row_count:1},truncated:false}});});
  await openPage(page,'database');await page.locator('.conn-list .conn-name',{hasText:conn.name}).click();await page.locator('.qe-edit .cm-content').fill(engine==='redis'?'GET synthetic:customer':'db.synthetic_customers.find({})');await page.getByRole('button',{name:/^Run\s+⌘/}).click();
  await page.getByRole('tab',{name:'Grid',exact:true}).click();await expect(page.locator('.gt-edit-hint')).toBeVisible();
  if(engine==='redis'){await page.getByText('Original value',{exact:true}).dblclick();await page.locator('.cell-input').fill('Updated value');await page.locator('.cell-input').press('Enter');}
  else {await page.locator('.cell.json').first().click();await page.locator('.cell-viewer').getByRole('button',{name:'Edit',exact:true}).click();await page.locator('.cv-edit').fill('[{"qty":2}]');await page.locator('.cell-viewer').getByRole('button',{name:/^Save/}).click();}
  await page.getByTestId('pending-edits-bar').getByRole('button',{name:'Review & apply',exact:true}).click();await expect(page.locator('.review-modal')).toBeVisible();
  const command=await page.locator('.review-sql').inputValue();expect(command).toContain(engine==='redis'?'SET synthetic:customer "Updated value"':'updateOne');if(engine==='mongodb')expect(command).toContain('"qty":2');
  mkdirSync(shots,{recursive:true});await page.screenshot({animations:'disabled',path:`${shots}/${engine}-review.png`});await page.locator('.review-modal').getByRole('button',{name:'Cancel',exact:true}).click();expect(queries).toBe(1);
});
