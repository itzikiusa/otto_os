import {test, expect, type APIRequestContext} from '@playwright/test';
import {readFileSync, mkdtempSync, writeFileSync, rmSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join} from 'node:path';
import {execFileSync} from 'node:child_process';
import {apiCtx, seedWorkspace} from './seed';

// R02 review regressions: real UI/isolated daemon; only fixture-owned sessions.
test.use({serviceWorkers:'block'});
let ctx:APIRequestContext, base='', workspace='', fixture='';
let sessions:string[]=[];
test.beforeEach(async()=>{({ctx,base}=await apiCtx());workspace=await seedWorkspace(ctx,base);fixture=mkdtempSync(join(tmpdir(),'otto-r02-'));});
test.afterEach(async()=>{for(const id of sessions)await ctx.delete(`${base}/api/v1/sessions/${id}`);sessions=[];await ctx.dispose();rmSync(fixture,{recursive:true,force:true});});
async function session(title:string,meta:Record<string,unknown>={}) {
 const r=await ctx.post(`${base}/api/v1/workspaces/${workspace}/sessions`,{data:{kind:'agent',provider:'shell',title,cwd:fixture,meta:{origin:'e2e',...meta}}});
 expect(r.ok(),await r.text()).toBe(true);const id=(await r.json()).id as string;sessions.push(id);return id;
}

async function dormantChat() {
 const path=join(fixture,'r02-transcript.jsonl');writeFileSync(path,JSON.stringify({type:'user',uuid:'r02-turn',message:{role:'user',content:'Recorded history remains readable'}})+'\n');
 const id=await session('R02 dormant chat',{nested_provider:'claude',e2e_transcript_path:path});
 expect((await ctx.post(`${base}/api/v1/sessions/${id}/kill`)).ok()).toBe(true);
 await expect.poll(async()=>(await(await ctx.get(`${base}/api/v1/sessions/${id}`)).json()).live).toBe(false);
 // Seed the exact suspended state in this run's temporary DB, never the real daemon.
 const meta=JSON.parse(readFileSync(join(process.cwd(),'e2e',`.auth-${process.env.OTTO_E2E_SLOT ?? '0'}`,'daemon.json'),'utf8')) as {dataDir:string};
 expect(meta.dataDir).toContain('otto-e2e-');
 execFileSync('sqlite3',[join(meta.dataDir,'otto.db'),`UPDATE sessions SET status='reconnectable',provider_session_id='r02-nested' WHERE id='${id}';`]);
 return id;
}

test('a suspended Chat tile stays dormant until an explicit resume',async({page})=>{
 const id=await dormantChat();
 await page.addInitScript(({workspace,id})=>{localStorage.setItem('otto_workspace',workspace);localStorage.setItem('otto_firstrun_dismissed','1');localStorage.setItem('otto_nav_all_ws','0');localStorage.setItem('otto_right_open','0');localStorage.setItem('otto_view_mode','tiled');localStorage.setItem(`otto_session_view:${id}`,'chat');},{workspace,id});
 await page.goto('/#/agents');
 await expect(page.locator('.conv[data-loaded="true"]')).toBeVisible();
 await expect(page.locator('.conv')).toContainText('Recorded history remains readable');
 await page.waitForTimeout(1500); // ui-guards: allow — allow an unintended auto-resume time to start before proving it stays dormant.
 const state=await(await ctx.get(`${base}/api/v1/sessions/${id}`)).json();
 expect(state.live,'view-only Chat tile must not spawn the shell').toBe(false);
 // Recovery/keepalive requests must stay passive too, not only the first read.
 for(let i=0;i<2;i++) expect((await ctx.post(`${base}/api/v1/sessions/${id}/transcript/touch?view=true`)).status()).toBe(204);
 const recovered=page.waitForResponse(r=>r.url().includes(`/sessions/${id}/transcript/touch?view=true`));
 await page.evaluate(()=>{
   Object.defineProperty(document,'hidden',{configurable:true,value:true});
   document.dispatchEvent(new Event('visibilitychange'));
   Object.defineProperty(document,'hidden',{configurable:true,value:false});
   document.dispatchEvent(new Event('visibilitychange'));
 });
 await recovered;
 expect((await(await ctx.get(`${base}/api/v1/sessions/${id}`)).json()).live).toBe(false);
 await page.locator('.composer').getByRole('button',{name:'Resume',exact:true}).click();
 await expect.poll(async()=>(await(await ctx.get(`${base}/api/v1/sessions/${id}`)).json()).live).toBe(true);
});

test('a delayed clipboard read does not paste into a replacement session',async({page})=>{
 const a=await session('R02 clipboard A'),b=await session('R02 clipboard B');
 await page.addInitScript(({workspace})=>{localStorage.setItem('otto_workspace',workspace);localStorage.setItem('otto_firstrun_dismissed','1');localStorage.setItem('otto_nav_all_ws','0');localStorage.setItem('otto_right_open','0');localStorage.setItem('otto_view_mode','split');},{workspace});
 await page.goto(`/#/agents/${a}`);
 await expect(page.locator('.pane-title').first()).toContainText('R02 clipboard A');
 await page.evaluate(()=>{const target=window as unknown as {releaseR02Clipboard:(s:string)=>void;r02ClipboardCalled:boolean};target.r02ClipboardCalled=false;Object.defineProperty(navigator.clipboard,'readText',{configurable:true,value:()=>{target.r02ClipboardCalled=true;return new Promise<string>(resolve=>{target.releaseR02Clipboard=resolve;});}});});
 const input=page.locator('.xterm-helper-textarea').first();await input.focus();await page.keyboard.press('Control+Shift+V');
 await expect.poll(()=>page.evaluate(()=>(window as unknown as {r02ClipboardCalled:boolean}).r02ClipboardCalled)).toBe(true);
 await page.evaluate(id=>{location.hash=`#/agents/${id}`;},b);
 await expect(page.locator('.pane-title').first()).toContainText('R02 clipboard B');
 await expect(page.locator('.xterm-helper-textarea').first()).toBeAttached();
 await page.evaluate(()=>(window as unknown as {releaseR02Clipboard:(s:string)=>void}).releaseR02Clipboard('R02_CLIPBOARD_FOR_A'));
 await page.waitForTimeout(700); // ui-guards: allow — let a stale paste reach the PTY before proving its absence.
 const screen=await(await ctx.get(`${base}/api/v1/sessions/${b}/screen`)).json();
 expect(JSON.stringify(screen),'clipboard requested in A must not reach B').not.toContain('R02_CLIPBOARD_FOR_A');
 // The ownership check must still permit a new paste requested in B.
 await page.evaluate(()=>{(window as unknown as {r02ClipboardCalled:boolean}).r02ClipboardCalled=false;});
 await page.locator('.xterm-helper-textarea').first().focus();await page.keyboard.press('Control+Shift+V');
 await expect.poll(()=>page.evaluate(()=>(window as unknown as {r02ClipboardCalled:boolean}).r02ClipboardCalled)).toBe(true);
 await page.evaluate(()=>(window as unknown as {releaseR02Clipboard:(s:string)=>void}).releaseR02Clipboard('R02_CLIPBOARD_FOR_B'));
 await expect.poll(async()=>JSON.stringify(await(await ctx.get(`${base}/api/v1/sessions/${b}/screen`)).json())).toContain('R02_CLIPBOARD_FOR_B');
});

// Active panes retain the established resume-on-open contract.
test('an active Chat pane resumes on open',async({page})=>{
 const id=await dormantChat();
 await page.addInitScript(({workspace,id})=>{localStorage.setItem('otto_workspace',workspace);localStorage.setItem('otto_firstrun_dismissed','1');localStorage.setItem('otto_nav_all_ws','0');localStorage.setItem('otto_right_open','0');localStorage.setItem('otto_view_mode','split');localStorage.setItem(`otto_session_view:${id}`,'chat');},{workspace,id});
 await page.goto(`/#/agents/${id}`);
 await expect(page.locator('.conv[data-loaded="true"]')).toBeVisible();
 await expect.poll(async()=>(await(await ctx.get(`${base}/api/v1/sessions/${id}`)).json()).live).toBe(true);
});
