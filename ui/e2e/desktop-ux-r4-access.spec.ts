import { test, expect, type Page, type Route } from '@playwright/test';
import AxeBuilder from '@axe-core/playwright';
import { expectFullyInViewport } from './helpers';
test.use({serviceWorkers:'block'});
const user={id:'fixture-admin',username:'root',display_name:'Fixture owner',is_root:true,disabled:false,created_at:'2026-09-25T00:00:00Z'};
const workspace=(id:string)=>({id,name:`Synthetic ${id}`,root_path:'/tmp/synthetic',role:'admin',created_at:'2026-09-25T00:00:00Z'});
function held() {let release!:(r:Route)=>void; const promise=new Promise<Route>(r=>release=r);return {promise,hold:(r:Route)=>release(r)};}
async function setup(page:Page) {
  await page.addInitScript(()=>{localStorage.removeItem('otto_token');localStorage.setItem('otto_firstrun_dismissed','1');});
  await page.route('**/api/v1/meta',r=>r.fulfill({json:{needs_onboarding:true,tools:[],version:'fixture'}}));
  await page.route('**/api/v1/auth/me',r=>r.fulfill({json:{user,real_user:user}}));
  await page.route('**/api/v1/auth/capabilities',r=>r.fulfill({json:{capabilities:{}}}));
  await page.route('**/api/v1/workspaces/*/sessions',r=>r.fulfill({json:[]}));
  await page.goto('/#/home');await page.getByRole('button',{name:'Get Started'}).click();
  await page.getByLabel('Password',{exact:true}).fill('Fixture-password-123');await page.getByLabel('Confirm password').fill('Fixture-password-123');await page.getByRole('button',{name:'Continue',exact:true}).click();
}
async function finishSteps(page:Page) {await page.getByRole('button',{name:'Continue',exact:true}).click();await page.getByRole('button',{name:'Finish Setup'}).click();}

test('first account Back then Skip honors the skipped workspace after failure',async({page})=>{
  let roots=0;let writes=0;
  await page.route('**/api/v1/onboarding/root',r=>{roots++;return r.fulfill({json:{token:'fixture-root',user}});});
  await page.route('**/api/v1/workspaces',r=>{if(r.request().method()==='GET')return r.fulfill({json:[]});writes++;return r.fulfill({status:503,json:{code:'upstream',message:'Synthetic workspace unavailable'}});});
  await setup(page);await page.getByLabel('Name',{exact:true}).fill('Fixture');await page.getByLabel('Directory',{exact:true}).fill('/tmp/fixture');await page.getByRole('button',{name:'Continue',exact:true}).click();await finishSteps(page);
  await expect(page.getByRole('alert')).toContainText('Synthetic workspace unavailable');
  await page.getByRole('button',{name:'Back',exact:true}).click();await page.getByRole('button',{name:'Back',exact:true}).click();
  await page.getByRole('button',{name:'Skip',exact:true}).click();await finishSteps(page);
  await expect(page.locator('.shell')).toBeVisible();expect(roots).toBe(1);expect(writes).toBe(1);
});

test('first account recovers a lost root response with entered credentials',async({page})=>{
  let roots=0;let logins=0;
  await page.route('**/api/v1/onboarding/root',r=>++roots===1?r.abort('connectionfailed'):r.fulfill({status:409,json:{code:'conflict',message:'already onboarded'}}));
  await page.route('**/api/v1/auth/login',r=>{expect(r.request().postDataJSON()).toEqual({username:'root',password:'Fixture-password-123'});logins++;return r.fulfill({json:{token:'fixture-recovered',user}});});
  await page.route('**/api/v1/workspaces',r=>r.fulfill({json:[]}));
  await setup(page);await page.getByRole('button',{name:'Skip',exact:true}).click();await finishSteps(page);
  // An ambiguous failure can recover immediately; if it remains visible, Retry
  // must recover the existing account instead of trapping users in a 409 loop.
  if(await page.getByRole('alert').isVisible())await page.getByRole('button',{name:'Finish Setup'}).click();
  await expect(page.locator('.shell')).toBeVisible();expect(logins).toBe(1);expect(roots).toBeLessThanOrEqual(2);
});

test('late workspace list from an old identity cannot replace the current workspace',async({page})=>{
  await page.addInitScript(()=>localStorage.setItem('otto_firstrun_dismissed','1'));
  await page.route('**/api/v1/workspaces',r=>r.fulfill({json:[workspace('initial')]}));
  await page.route('**/api/v1/workspaces/scratch',r=>r.fulfill({json:workspace('scratch')}));
  await page.route('**/api/v1/workspaces/*/sessions',r=>r.fulfill({json:[]}));
  await page.route('**/api/v1/auth/me',r=>r.fulfill({json:{user,real_user:user}}));
  await page.goto('/#/agents');await expect(page.locator('.shell')).toBeVisible();
  await expect.poll(()=>page.evaluate(async()=>{const p='/src/lib/stores/workspace.svelte.ts';return (await import(p)).ws.currentId;})).toBe('initial');
  const old=held();await page.route('**/api/v1/workspaces',r=>r.request().headers().authorization==='Bearer fixture-new'?r.fulfill({json:[workspace('new')]}):old.hold(r));
  await page.evaluate(async()=>{const p='/src/lib/stores/workspace.svelte.ts';void (await import(p)).ws.load();});const pending=await old.promise;
  await page.evaluate(async()=>{const p='/src/lib/api/client.ts';const s='/src/lib/stores/workspace.svelte.ts';(await import(p)).setToken('fixture-new');await (await import(s)).ws.load();});
  await pending.fulfill({json:[workspace('old')]});
  await expect.poll(()=>page.evaluate(async()=>{const p='/src/lib/stores/workspace.svelte.ts';const {ws}=await import(p);return {current:ws.currentId,list:ws.workspaces.map((w:{id:string})=>w.id)};})).toEqual({current:'new',list:['new']});
});

for(const unavailable of ['removed','inert']) test(`nested sheet restores usable parent focus when trigger becomes ${unavailable}`,async({page})=>{
  await page.addInitScript(()=>localStorage.setItem('otto_firstrun_dismissed','1'));
  await page.goto('/#/agents');await expect(page.locator('.shell')).toBeVisible();await page.keyboard.press('Meta+t');
  const parent=page.getByRole('dialog',{name:'New session',exact:true});const browse=parent.getByRole('button',{name:'Browse…'}).first();await browse.click();
  const picker=page.getByRole('dialog',{name:'Choose working directory'});await expect(picker).toBeVisible();
  await browse.evaluate((el,unavailable)=>{if(unavailable==='removed')el.remove();else el.setAttribute('inert','');},unavailable);
  await page.keyboard.press('Escape');await expect(picker).toBeHidden();
  await expect.poll(()=>parent.evaluate(el=>el.contains(document.activeElement)&&!(document.activeElement as HTMLElement)?.closest('[inert]'))).toBe(true);
});

test('a capped sheet updates its keyboard scrolling stop after delayed intrinsic image size',async({page})=>{
  await page.addInitScript(()=>localStorage.setItem('otto_firstrun_dismissed','1'));
  await page.goto('/#/agents');await expect(page.locator('.shell')).toBeVisible();
  const image=held();await page.route('**/synthetic-delayed-image.svg',image.hold);
  await page.evaluate(async()=>{const p='/src/lib/confirm.svelte.ts';void (await import(p)).confirmer.ask('Synthetic preview',{title:'Preview'});});
  const body=page.getByRole('dialog',{name:'Preview',exact:true}).locator('.sheet-body');
  // Embedded previews/editors may allocate their own fixed body viewport.
  // An image then changes scrollHeight without resizing that viewport.
  await body.evaluate(el=>{const node=el as HTMLElement;node.style.height='180px';node.style.flex='0 0 180px';const img=document.createElement('img');img.alt='Synthetic tall preview';img.src='/synthetic-delayed-image.svg';node.append(img);});
  const pending=await image.promise;await expect(body).not.toHaveAttribute('tabindex','0');
  await pending.fulfill({contentType:'image/svg+xml',body:'<svg xmlns="http://www.w3.org/2000/svg" width="120" height="900"><rect width="120" height="900" fill="gray"/></svg>'});
  await expect.poll(()=>body.evaluate(el=>el.scrollHeight-el.clientHeight)).toBeGreaterThan(500);
  await expect(body).toHaveAttribute('tabindex','0');
});

test('first-account steps announce their heading and associate password validation',async({page})=>{
  await page.addInitScript(()=>localStorage.removeItem('otto_token'));
  await page.route('**/api/v1/meta',r=>r.fulfill({json:{needs_onboarding:true,tools:[],version:'fixture'}}));
  await page.goto('/#/home');await page.getByRole('button',{name:'Get Started'}).focus();await page.keyboard.press('Enter');
  await expect(page.getByRole('heading',{name:'Set the root password'})).toBeFocused();
  await page.getByLabel('Password',{exact:true}).fill('Fixture-password-123');await page.getByLabel('Confirm password').fill('Mismatch');
  await expect(page.getByLabel('Confirm password')).toHaveAttribute('aria-invalid','true');
  await expect(page.getByRole('status')).toContainText("Passwords don't match");
  await page.getByLabel('Confirm password').fill('Fixture-password-123');await page.getByRole('button',{name:'Continue',exact:true}).click();
  await expect(page.getByRole('heading',{name:'Create your first workspace'})).toBeFocused();
  await page.getByRole('button',{name:'Skip',exact:true}).click();await expect(page.getByRole('heading',{name:'Usage tracking'})).toBeFocused();
  await page.getByRole('button',{name:'Continue',exact:true}).click();await expect(page.getByRole('heading',{name:'Tool check'})).toBeFocused();
});

async function readable(page:Page,selector:string) {
  return page.locator(selector).evaluateAll(nodes=>{
    type C=number[];
    const parse=(s:string):C=>{let m=/rgba?\(([^)]+)\)/.exec(s);if(m){const p=m[1].split(/[\s,/]+/).filter(Boolean).map(Number);return [p[0]/255,p[1]/255,p[2]/255,p[3]??1];}m=/color\(srgb ([^)]+)\)/.exec(s);if(m){const p=m[1].split(/[\s/]+/).filter(Boolean).map(Number);return [p[0],p[1],p[2],p[3]??1];}if(!CSS.supports('color',s))throw new Error(`Unsupported computed color ${s}`);const canvas=document.createElement('canvas');canvas.width=canvas.height=1;const ctx=canvas.getContext('2d')!;ctx.fillStyle=s;ctx.fillRect(0,0,1,1);return Array.from(ctx.getImageData(0,0,1,1).data).map(v=>v/255);};
    const lum=(c:C)=>{const l=c.map(v=>v<=0.04045?v/12.92:((v+0.055)/1.055)**2.4);return .2126*l[0]+.7152*l[1]+.0722*l[2];};
    return nodes.filter(el=>{const r=el.getBoundingClientRect();return r.width>0&&r.height>0&&r.bottom>0&&r.top<innerHeight&&!el.closest('[aria-hidden="true"]')&&getComputedStyle(el).visibility!=='hidden'&&(el.textContent?.trim()||el instanceof HTMLInputElement);}).map(el=>{
      const chain:Element[]=[];for(let p:Element|null=el;p;p=p.parentElement)chain.unshift(p);let bg:C=[1,1,1];
      for(const p of chain){const c=parse(getComputedStyle(p).backgroundColor);bg=bg.map((v,i)=>c[i]*c[3]+v*(1-c[3]));}
      const s=getComputedStyle(el),c=parse(s.color),f=bg.map((v,i)=>c[i]*c[3]+v*(1-c[3]));const a=lum(f),b=lum(bg);
      return {text:el.textContent?.trim().slice(0,70)||'input',font:parseFloat(s.fontSize),ratio:(Math.max(a,b)+.05)/(Math.min(a,b)+.05)};
    });
  });
}
for(const v of [{theme:'native',scheme:'light',width:375},{theme:'native',scheme:'dark',width:1000},{theme:'warm',scheme:'light',width:375},{theme:'warm',scheme:'dark',width:834},{theme:'pro-dark',scheme:'dark',width:1024}])test(`loaded shared sheet and setup contrast ${v.theme}-${v.scheme}`,async({page},info)=>{
  await page.setViewportSize({width:v.width,height:720});await page.emulateMedia({reducedMotion:'reduce'});
  await page.addInitScript(v=>{localStorage.setItem('otto_theme',v.theme);localStorage.setItem('otto_scheme',v.scheme);localStorage.setItem('otto_direction','rtl');localStorage.setItem('otto_accent',v.scheme==='light'?'#ffff00':'#050505');localStorage.setItem('otto_reduce_transparency','1');localStorage.setItem('otto_ambient','wallpaper');},v);
  await setup(page);await page.getByLabel('Name',{exact:true}).fill('Synthetic workspace');await page.getByLabel('Directory',{exact:true}).fill('/tmp/synthetic/review');
  const samples=await readable(page,'.ob-card h1,.ob-card p,.ob-card label,.ob-card input,.ob-card button');
  for(const sample of samples){expect(sample.font,sample.text).toBeGreaterThanOrEqual(11);expect(sample.ratio,sample.text).toBeGreaterThanOrEqual(4.5);}
  await page.screenshot({path:info.outputPath('setup-custom-accent.png')});
  expect((await new AxeBuilder({page}).include('.ob-card').withTags(['wcag2a','wcag2aa','wcag21aa']).analyze()).violations).toEqual([]);
  await page.route('**/api/v1/onboarding/root',r=>r.fulfill({json:{token:'fixture-root',user}}));
  await page.route('**/api/v1/workspaces',r=>r.fulfill({json:r.request().method()==='POST'?workspace('preview'):[]}));
  await page.getByRole('button',{name:'Continue',exact:true}).click();await finishSteps(page);await expect(page.locator('.shell')).toBeVisible();
  await page.evaluate(async()=>{const p='/src/lib/confirm.svelte.ts';void (await import(p)).confirmer.choose('Review the synthetic workspace actions and keep the draft available.',{title:'Review workspace',options:[{label:'Apply draft',value:'apply',kind:'primary'},{label:'Discard draft',value:'discard',kind:'danger'}]});});
  const sheet=page.getByRole('dialog',{name:'Review workspace'});await expect(sheet).toBeVisible();
  const measurements=await readable(page,'.sheet h2,.sheet p,.sheet footer button');
  for(const sample of measurements){expect(sample.font,sample.text).toBeGreaterThanOrEqual(11);expect(sample.ratio,sample.text).toBeGreaterThanOrEqual(4.5);}
  await expectFullyInViewport(page,sheet);expect(await sheet.evaluate(el=>parseFloat(getComputedStyle(el).animationDuration))).toBeLessThanOrEqual(.001);
  await page.screenshot({path:info.outputPath('sheet-custom-accent.png')});
  await page.evaluate(async()=>{const p='/src/lib/stores/ui.svelte.ts';(await import(p)).ui.setReduceTransparency(false);});
  await expect.poll(()=>page.evaluate(()=>getComputedStyle(document.querySelector('.shell')!).backgroundImage)).not.toBe('none');
  for(const sample of await readable(page,'.sheet h2,.sheet p,.sheet footer button'))expect(sample.ratio,sample.text).toBeGreaterThanOrEqual(4.5);
  await page.screenshot({path:info.outputPath('sheet-wallpaper.png')});
  await page.evaluate(async()=>{const p='/src/lib/stores/ui.svelte.ts';(await import(p)).ui.setReduceTransparency(true);});
  await expect.poll(()=>page.evaluate(()=>getComputedStyle(document.querySelector('.shell')!).backgroundImage)).toBe('none');
  await info.attach('visible-computed-text.json',{body:JSON.stringify({setup:samples,sheet:measurements},null,2),contentType:'application/json'});
});

test('effective-user change reloads workspaces without carrying the previous list',async({page})=>{
  let changed=false;
  await page.addInitScript(()=>localStorage.setItem('otto_firstrun_dismissed','1'));
  await page.route('**/api/v1/auth/me',r=>r.fulfill({json:{user:changed?{...user,id:'fixture-guest',is_root:false}:user,real_user:user}}));
  await page.route('**/api/v1/auth/capabilities',r=>r.fulfill({json:{capabilities:{agents:'view'}}}));
  await page.route('**/api/v1/admin/impersonate/fixture-guest',r=>{changed=true;return r.fulfill({json:{token:'fixture-new'}});});
  await page.route('**/api/v1/workspaces',r=>r.fulfill({json:[workspace(changed?'new':'old')]}));
  await page.route('**/api/v1/workspaces/*/sessions',r=>r.fulfill({json:[]}));
  await page.goto('/#/agents');await expect(page.locator('.shell')).toBeVisible();
  await expect.poll(()=>page.evaluate(async()=>{const p='/src/lib/stores/workspace.svelte.ts';return (await import(p)).ws.currentId;})).toBe('old');
  await page.evaluate(async()=>{const p='/src/lib/stores/auth.svelte.ts';await (await import(p)).auth.impersonate('fixture-guest');});
  await expect.poll(()=>page.evaluate(async()=>{const p='/src/lib/stores/workspace.svelte.ts';const {ws}=await import(p);return {current:ws.currentId,list:ws.workspaces.map((w:{id:string})=>w.id)};})).toEqual({current:'new',list:['new']});
});

for(const v of [{theme:'native',scheme:'light',accent:'#ffff00'},{theme:'native',scheme:'dark',accent:'#050505'},{theme:'warm',scheme:'light',accent:'#ffff00'},{theme:'warm',scheme:'dark',accent:'#050505'},{theme:'pro-dark',scheme:'dark',accent:'#050505'}])test(`custom accent keeps actual Markdown links and focus visible ${v.theme}-${v.scheme}`,async({page},info)=>{
  await page.addInitScript(v=>{localStorage.setItem('otto_theme',v.theme);localStorage.setItem('otto_scheme',v.scheme);localStorage.setItem('otto_accent',v.accent);localStorage.setItem('otto_firstrun_dismissed','1');},v);
  await page.goto('/#/agents');await expect(page.locator('.shell')).toBeVisible();
  await page.evaluate(async()=>{const p='/src/lib/confirm.svelte.ts';void (await import(p)).confirmer.promptText('Synthetic editable preview',{title:'Read preview'});});
  const sheet=page.getByRole('dialog',{name:'Read preview',exact:true});await expect(sheet).toBeVisible();
  // Render the production Markdown primitive inside an actual sheet so its
  // inherited theme/background, anchor and input styles are measured together.
  await sheet.locator('.sheet-body').evaluate(async el=>{const p='/src/lib/md.ts';const md=document.createElement('article');md.className='md-body';md.innerHTML=(await import(p)).renderMarkdownGfm('[Read complete documentation](https://example.invalid/guide)');el.append(md);});
  const text=await readable(page,'.sheet .md-body a');
  for(const sample of text)expect.soft(sample.ratio,sample.text).toBeGreaterThanOrEqual(4.5);
  // Reuse the same compositing calculation for the visible focus border.
  await sheet.locator('input').focus();
  const finalColor=await sheet.locator('.md-body a').evaluate(el=>getComputedStyle(el).color);
  await expect.poll(()=>sheet.locator('input').evaluate(el=>getComputedStyle(el).borderColor)).toBe(finalColor);
  await sheet.locator('input').evaluate(el=>{(el as HTMLElement).style.color=getComputedStyle(el).borderColor;});
  const border=await readable(page,'.sheet input');expect.soft(border[0].ratio,'Input focus boundary').toBeGreaterThanOrEqual(3);
  await sheet.locator('input').evaluate(el=>(el as HTMLElement).style.removeProperty('color'));
  await page.screenshot({path:info.outputPath('accent-link-focus.png')});
});

test('late cross-workspace sessions cannot repopulate another effective identity',async({page})=>{
  let changed=false;const old=held();
  await page.addInitScript(()=>localStorage.setItem('otto_firstrun_dismissed','1'));
  await page.route('**/api/v1/auth/me',r=>r.fulfill({json:{user:changed?{...user,id:'fixture-guest',is_root:false}:user,real_user:user}}));
  await page.route('**/api/v1/auth/capabilities',r=>r.fulfill({json:{capabilities:{agents:'view'}}}));
  await page.route('**/api/v1/admin/impersonate/fixture-guest',r=>{changed=true;return r.fulfill({json:{token:'fixture-new'}});});
  await page.route('**/api/v1/workspaces',r=>r.fulfill({json:changed?[workspace('new')]:[workspace('old'),workspace('other')]}));
  await page.route('**/api/v1/workspaces/*/sessions',r=>r.request().url().includes('/other/')?old.hold(r):r.fulfill({json:[]}));
  await page.goto('/#/agents');const pending=await old.promise;
  await page.evaluate(async()=>{const p='/src/lib/stores/auth.svelte.ts';await (await import(p)).auth.impersonate('fixture-guest');});
  await expect.poll(()=>page.evaluate(async()=>{const p='/src/lib/stores/workspace.svelte.ts';return (await import(p)).ws.currentId;})).toBe('new');
  await pending.fulfill({json:[{id:'old-private',workspace_id:'other',title:'Old identity private session',status:'idle',provider:'shell',cwd:'/tmp/synthetic',meta:{},last_active_at:'2026-09-25T00:00:00Z'}]});
  await expect.poll(()=>page.evaluate(async()=>{const p='/src/lib/stores/workspace.svelte.ts';return (await import(p)).ws.otherWsSessions.map((s:{id:string})=>s.id);})).toEqual([]);
  await expect(page.getByText('Old identity private session',{exact:true})).toBeHidden();
});

test('workspace directory stays left-to-right in RTL setup',async({page})=>{
  await page.addInitScript(()=>localStorage.setItem('otto_direction','rtl'));await setup(page);
  const path=page.getByLabel('Directory',{exact:true});await path.fill('/tmp/synthetic/project');await expect(path).toHaveCSS('direction','ltr');
});

test('late scratch discovery preserves a newly created selected workspace',async({page})=>{
  await page.addInitScript(()=>localStorage.setItem('otto_firstrun_dismissed','1'));
  await page.route('**/api/v1/workspaces',r=>r.fulfill({json:r.request().method()==='POST'?workspace('chosen'):[workspace('initial')]}));
  await page.route('**/api/v1/workspaces/scratch',r=>r.fulfill({json:workspace('scratch')}));
  await page.route('**/api/v1/workspaces/*/sessions',r=>r.fulfill({json:[]}));
  await page.goto('/#/agents');
  await expect.poll(()=>page.evaluate(async()=>{const p='/src/lib/stores/workspace.svelte.ts';return (await import(p)).ws.currentId;})).toBe('initial');
  const scratch=held();await page.route('**/api/v1/workspaces/scratch',scratch.hold);
  await page.evaluate(async()=>{const p='/src/lib/stores/workspace.svelte.ts';void (await import(p)).ws.load();});const pending=await scratch.promise;
  await page.evaluate(async()=>{const p='/src/lib/stores/workspace.svelte.ts';await (await import(p)).ws.createWorkspace('Chosen','/tmp/synthetic');});
  await pending.fulfill({json:workspace('scratch')});
  await expect.poll(()=>page.evaluate(async()=>{const p='/src/lib/stores/workspace.svelte.ts';return (await import(p)).ws.currentId;})).toBe('chosen');
});
