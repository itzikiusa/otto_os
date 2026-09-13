import {test, expect, request, type APIRequestContext} from '@playwright/test';
import {randomUUID} from 'node:crypto';
import {appendFileSync, mkdtempSync, rmSync, writeFileSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join} from 'node:path';
import {apiCtx, seedWorkspace} from './seed';

// Real HTTP/cache/auth/WS integration. The only child is an inert shell PTY;
// synthetic JSONL belongs to this test, and no coding provider is invoked.
let root:APIRequestContext, base='', token='', workspace='', session='', fixtureDir='';
let transcriptPath='';
const sentinel='isolated cached transcript content';
function record(id:string,text:string) {return JSON.stringify({type:'user',uuid:id,sessionId:'00000000-0000-4000-8000-000000000123',timestamp:'2026-09-13T12:00:00Z',message:{role:'user',content:text}})+'\n';}

test.beforeEach(async({},info)=>{
  test.skip(info.project.name!=='desktop-browser','desktop browser project only');
  ({ctx:root,base,token}=await apiCtx());workspace=await seedWorkspace(root,base);
  fixtureDir=mkdtempSync(join(tmpdir(),'otto-transcript-cache-fixture-'));transcriptPath=join(fixtureDir,'00000000-0000-4000-8000-000000000123.jsonl');
  writeFileSync(transcriptPath,record('initial',sentinel));
  const created=await root.post(`${base}/api/v1/workspaces/${workspace}/sessions`,{data:{kind:'agent',provider:'shell',title:'Isolated transcript reader',cwd:fixtureDir,meta:{origin:'e2e',nested_provider:'claude',e2e_transcript_path:transcriptPath}}});
  expect(created.ok(),await created.text()).toBeTruthy();session=(await created.json()).id;
  await expect.poll(async()=> (await (await root.get(`${base}/api/v1/sessions/${session}`)).json()).live).toBe(true);
});
test.afterEach(async({page})=>{
  await page.evaluate(()=>{(globalThis as unknown as {fixtureSocket?:WebSocket}).fixtureSocket?.close();}).catch(()=>{});
  if(session && root){const response=await root.delete(`${base}/api/v1/sessions/${session}`);expect(response.ok(),await response.text()).toBeTruthy();}
  session='';await root?.dispose();if(fixtureDir)rmSync(fixtureDir,{recursive:true,force:true});fixtureDir='';
});

test('warm transcript cache and History cursors recheck changed authorization',async()=>{
  const username=`transcript-reader-${randomUUID()}`,password='fixture-password-12345';
  const userResponse=await root.post(`${base}/api/v1/users`,{data:{username,password,display_name:'Isolated transcript reader'}});
  expect(userResponse.ok(),await userResponse.text()).toBeTruthy();const user=(await userResponse.json()).id;
  const grant=await root.put(`${base}/api/v1/users/${user}/grants`,{data:{grants:[{feature:'agents',capability:'view'}]}});expect(grant.ok(),await grant.text()).toBeTruthy();
  async function member(role:'admin'|'viewer'|null){const r=await root.put(`${base}/api/v1/workspaces/${workspace}/members`,{data:{members:role?[{user_id:user,role}]:[]}});expect(r.ok(),await r.text()).toBeTruthy();}
  await member('admin');
  const logged=await root.post(`${base}/api/v1/auth/login`,{data:{username,password}});expect(logged.ok(),await logged.text()).toBeTruthy();
  const reader=await request.newContext({extraHTTPHeaders:{Authorization:`Bearer ${(await logged.json()).token}`}});
  let second='';
  try {
    const created=await root.post(`${base}/api/v1/workspaces/${workspace}/sessions`,{data:{kind:'agent',provider:'shell',title:'Second isolated History reader',cwd:fixtureDir,meta:{origin:'e2e',nested_provider:'claude',e2e_transcript_path:transcriptPath}}});
    expect(created.ok(),await created.text()).toBeTruthy();second=(await created.json()).id;
    const historyUrl=`${base}/api/v1/workspaces/${workspace}/history/page?limit=1&cwd=${encodeURIComponent(fixtureDir)}`;
    const firstPage=await reader.get(historyUrl);expect(firstPage.status()).toBe(200);const firstBody=await firstPage.json();
    expect(firstBody.entries).toHaveLength(1);expect(typeof firstBody.next_cursor).toBe('string');
    const continuation=`${historyUrl}&cursor=${encodeURIComponent(firstBody.next_cursor)}`;
    const nextPage=await reader.get(continuation);expect(nextPage.status()).toBe(200);const nextBody=await nextPage.json();
    expect(nextBody.entries).toHaveLength(1);expect(nextBody.entries[0].session_id).not.toBe(firstBody.entries[0].session_id);expect(nextBody.next_cursor).toBeNull();
    for(const response of [await reader.get(`${continuation}&q=changed-filter`),await root.get(continuation),await reader.get(`${historyUrl}&cursor=malformed`)]){
      expect(response.status()).toBe(400);expect(await response.text()).not.toContain(fixtureDir);
    }
    const url=`${base}/api/v1/sessions/${session}/transcript?limit=60`;
    // Same path, provider, stat and options: the second read uses the retained
    // entry. The cache unit test separately counts the actual fold invocation.
    for(let n=0;n<2;n++){const r=await reader.get(url);expect(r.status()).toBe(200);expect(await r.text()).toContain(sentinel);}
    await member('viewer'); // Workspace access remains, session ownership does not.
    const changedRole=await reader.get(continuation);expect(changedRole.status()).toBe(400);expect(await changedRole.text()).not.toContain(fixtureDir);
    const ownOnly=await reader.get(historyUrl);expect(ownOnly.status()).toBe(200);expect((await ownOnly.json()).entries).toEqual([]);
    const noOwner=await reader.get(url);expect(noOwner.status()).toBe(403);expect(await noOwner.text()).toContain('not the session owner or a workspace admin');expect(await noOwner.text()).not.toContain(sentinel);
    await member(null);
    const revokedPage=await reader.get(continuation);expect(revokedPage.status()).toBe(403);expect(await revokedPage.text()).toContain('not a member of this workspace');expect(await revokedPage.text()).not.toContain(fixtureDir);
    const noWorkspace=await reader.get(url);expect(noWorkspace.status()).toBe(403);expect(await noWorkspace.text()).toContain('not a member of this workspace');expect(await noWorkspace.text()).not.toContain(sentinel);
    const stillAllowed=await root.get(url);expect(stillAllowed.status()).toBe(200);expect(await stillAllowed.text()).toContain(sentinel);
  } finally {await reader.dispose();if(second){const removed=await root.delete(`${base}/api/v1/sessions/${second}`);expect(removed.ok(),await removed.text()).toBeTruthy();}}
});

test('first transcript GET on a running shell arms live append delivery without a touch request',async({page})=>{
  // Health document has no application scripts, so no UI GET/touch can arm the
  // tail behind this test. Connect the authenticated event stream before GET.
  await page.goto(`${base}/api/v1/health`);
  await page.evaluate(async({base,token})=>{
    const target=globalThis as unknown as {fixtureSocket:WebSocket;fixtureEvents:Array<Record<string,unknown>>};target.fixtureEvents=[];
    await new Promise<void>((resolve,reject)=>{
      const socket=new WebSocket(`${base.replace('http:','ws:')}/ws/events`,['otto-bearer',token]);target.fixtureSocket=socket;
      const deadline=setTimeout(()=>{socket.close();reject(new Error('fixture event stream did not open'));},5000);
      socket.onopen=()=>{clearTimeout(deadline);resolve();};socket.onerror=()=>{clearTimeout(deadline);reject(new Error('fixture event stream failed'));};
      socket.onmessage=e=>{target.fixtureEvents.push(JSON.parse(String(e.data)));};
    });
  },{base,token});
  const first=await root.get(`${base}/api/v1/sessions/${session}/transcript?limit=60`);expect(first.status()).toBe(200);expect(await first.text()).toContain(sentinel);
  // The first live-screen frame proves initial seeding completed. This avoids
  // a sleep-based append race with the tail's initial full fold/current offset.
  await expect.poll(()=>page.evaluate(id=>(globalThis as unknown as {fixtureEvents:Array<{type:string;session_id?:string}>}).fixtureEvents.some(e=>e.type==='transcript_live'&&e.session_id===id),session),{timeout:10_000}).toBe(true);
  const appended='new synthetic turn delivered through live tail';appendFileSync(transcriptPath,record('appended',appended));
  await expect.poll(()=>page.evaluate(({id,text})=>(globalThis as unknown as {fixtureEvents:Array<{type:string;session_id?:string}>}).fixtureEvents.some(e=>e.type==='transcript_appended'&&e.session_id===id&&JSON.stringify(e).includes(text)),{id:session,text:appended}),{timeout:10_000}).toBe(true);
  const refreshed=await root.get(`${base}/api/v1/sessions/${session}/transcript?limit=60`);expect(refreshed.status()).toBe(200);expect(await refreshed.text()).toContain(appended);
});
