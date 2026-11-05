import {test, expect, type APIRequestContext} from '@playwright/test';
import {mkdtempSync, rmSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join} from 'node:path';
import {apiCtx, seedWorkspace} from './seed';

// R02 review regressions: real UI/isolated daemon; only fixture-owned sessions.
// (The suspended/active Chat-pane cases went with the session Chat view.)
test.use({serviceWorkers:'block'});
let ctx:APIRequestContext, base='', workspace='', fixture='';
let sessions:string[]=[];
test.beforeEach(async()=>{({ctx,base}=await apiCtx());workspace=await seedWorkspace(ctx,base);fixture=mkdtempSync(join(tmpdir(),'otto-r02-'));});
test.afterEach(async()=>{for(const id of sessions)await ctx.delete(`${base}/api/v1/sessions/${id}`);sessions=[];await ctx.dispose();rmSync(fixture,{recursive:true,force:true});});
async function session(title:string,meta:Record<string,unknown>={}) {
 const r=await ctx.post(`${base}/api/v1/workspaces/${workspace}/sessions`,{data:{kind:'agent',provider:'shell',title,cwd:fixture,meta:{origin:'e2e',...meta}}});
 expect(r.ok(),await r.text()).toBe(true);const id=(await r.json()).id as string;sessions.push(id);return id;
}

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
