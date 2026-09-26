import {test, expect, type Page, type Route} from '@playwright/test';
import AxeBuilder from '@axe-core/playwright';
import {expectFullyInViewport} from './helpers';
test.use({serviceWorkers:'block'});
const user={id:'fixture-admin',username:'root',display_name:'Fixture owner',is_root:true,disabled:false,created_at:'2026-09-25T00:00:00Z'};
const meta={needs_onboarding:false,tools:[],version:'fixture'};
const session=(id:string)=>({id,title:`Synthetic ${id}`,status:'idle',provider:'shell',cwd:'/tmp/fixture',created_at:new Date().toISOString()});
async function guest(page:Page) {
  await page.addInitScript(()=>{localStorage.removeItem('otto_token');localStorage.setItem('otto_firstrun_dismissed','1');});
  await page.route('**/api/v1/meta',r=>r.fulfill({json:meta}));
  await page.routeWebSocket(/\/ws\/term\//,socket=>{
    socket.onMessage(raw=>{const message=JSON.parse(String(raw));if(message.type==='scrollback')socket.send(JSON.stringify({type:'scrollback',epoch:1,data:Buffer.from('Synthetic guest terminal\r\nRead-only review output\r\n').toString('base64')}));});
  });
}
async function navigate(page:Page,id:string) {await page.evaluate(id=>{location.hash=`#/s/${id}/token-${id}`;},id);}
function deferredRoute() {let release!:(r:Route)=>void; const promise=new Promise<Route>(resolve=>release=resolve);return {promise,hold:(r:Route)=>{release(r);}};}

test('guest navigation ignores old metadata and role replies',async({page})=>{
  await guest(page); const old=deferredRoute();
  await page.route('**/api/v1/sessions/old',old.hold);
  await page.route('**/api/v1/sessions/new',r=>r.fulfill({json:session('new')}));
  await page.route('**/api/v1/share/whoami',r=>r.fulfill({json:{session_id:'new',role:r.request().headers().authorization?.includes('old')?'editor':'viewer'}}));
  await page.goto('/#/s/old/token-old'); const pending=await old.promise;
  await navigate(page,'new'); await expect(page.locator('.session-title')).toHaveText('Synthetic new');
  await pending.fulfill({json:session('old')});
  await expect(page.locator('.session-title')).toHaveText('Synthetic new');
  await expect(page.getByText('You can type',{exact:true})).toBeHidden();
});

test('guest OTP reply cannot replace another share error or keep verification busy',async({page})=>{
  await guest(page); const verify=deferredRoute();
  await page.route('**/api/v1/sessions/old',r=>r.fulfill({status:403,json:{code:'forbidden',message:'share requires email-OTP verification'}}));
  await page.route('**/api/v1/sessions/new',r=>r.fulfill({status:410,json:{code:'gone',message:'Synthetic expired link'}}));
  await page.route('**/api/v1/share/verify',verify.hold);
  await page.goto('/#/s/old/token-old'); await page.getByLabel('One-time access code').fill('123456'); await page.getByRole('button',{name:'Verify',exact:true}).click();
  const pending=await verify.promise; await navigate(page,'new'); await expect(page.getByText('Couldn’t open this session')).toBeVisible();
  await pending.fulfill({status:401,json:{code:'unauthorized',message:'Old code expired'}});
  await navigate(page,'old'); await expect(page.getByRole('button',{name:'Verify',exact:true})).toBeEnabled();
  await expect(page.getByRole('alert')).toBeHidden();
});

test('guest access recheck cannot revoke a newer session',async({page})=>{
  await guest(page);let calls=0;const recheck=deferredRoute();
  await page.route('**/api/v1/sessions/old',r=>++calls===1?r.fulfill({json:session('old')}):recheck.hold(r));
  await page.route('**/api/v1/sessions/new',r=>r.fulfill({json:session('new')}));
  await page.route('**/api/v1/share/whoami',r=>r.fulfill({json:{session_id:'fixture',role:'viewer'}}));
  await page.goto('/#/s/old/token-old');await expect(page.locator('.session-title')).toHaveText('Synthetic old');
  await page.evaluate(()=>window.dispatchEvent(new Event('focus')));const pending=await recheck.promise;
  await navigate(page,'new');await expect(page.locator('.session-title')).toHaveText('Synthetic new');
  await pending.fulfill({status:403,json:{code:'forbidden',message:'share requires email-OTP verification'}});
  await expect(page.locator('.session-title')).toHaveText('Synthetic new');
});

test('capabilities fail closed after changing the effective identity',async({page})=>{
  await page.route('**/api/v1/meta',r=>r.fulfill({json:meta}));let impersonating=false;
  await page.route('**/api/v1/auth/me',r=>r.fulfill({json:{user:impersonating?{...user,id:'fixture-guest',is_root:false}:user,real_user:user}}));
  await page.route('**/api/v1/auth/capabilities',r=>r.fulfill(impersonating?{status:503,json:{code:'upstream',message:'Synthetic capability outage'}}:{json:{capabilities:{git:'admin',agents:'admin'}}}));
  await page.route('**/api/v1/admin/impersonate/fixture-guest',r=>{impersonating=true;return r.fulfill({json:{token:'fixture-impersonation'}});});
  await page.goto('/#/home');await expect(page.locator('.shell')).toBeVisible();
  const allowed=await page.evaluate(async()=>{const path='/src/lib/stores/auth.svelte.ts';const {auth}=await import(path);await auth.impersonate('fixture-guest');return {can:auth.can('git','admin'),identity:auth.me.id};});
  expect(allowed).toEqual({can:false,identity:'fixture-guest'});
});

test('offline automatic retry preserves focused recovery action',async({page},info)=>{
  await guest(page);let count=0;const retry=deferredRoute();
  await page.route('**/api/v1/meta',r=>++count===1?r.abort():retry.hold(r));
  await page.goto('/#/home');const button=page.getByRole('button',{name:'Retry now'});await expect(button).toBeVisible();await button.focus();
  const pending=await retry.promise;await expect(button).toBeVisible();await expect(button).toBeFocused();
  await expectFullyInViewport(page,page.locator('.boot-sub'));
  expect(await page.evaluate(()=>document.documentElement.scrollWidth-window.innerWidth)).toBeLessThanOrEqual(1);
  await page.screenshot({path:info.outputPath('offline.png')});await pending.fulfill({json:meta});await expect(page.getByLabel('Username')).toBeVisible();
});

for(const v of [{theme:'native',scheme:'light',width:375},{theme:'native',scheme:'dark',width:1000},{theme:'warm',scheme:'light',width:375},{theme:'warm',scheme:'dark',width:834},{theme:'pro-dark',scheme:'dark',width:1024}]) test(`login error readable and announced ${v.theme}-${v.scheme}`,async({page},info)=>{
  await guest(page);await page.setViewportSize({width:v.width,height:667});await page.emulateMedia({reducedMotion:'reduce'});
  await page.addInitScript(v=>{localStorage.setItem('otto_theme',v.theme);localStorage.setItem('otto_scheme',v.scheme);localStorage.setItem('otto_direction','rtl');},v);
  await page.route('**/api/v1/auth/login',r=>r.fulfill({status:401,json:{code:'unauthorized',message:'Synthetic invalid credentials'}}));
  await page.goto('/#/home');await page.getByLabel('Username').fill('fixture');await page.getByLabel('Password',{exact:true}).fill('wrong-password');await page.getByRole('button',{name:'Sign In',exact:true}).click();
  await expect(page.getByText('Wrong username or password.')).toBeVisible();
  await page.screenshot({path:info.outputPath('login.png')});
  expect((await new AxeBuilder({page}).include('.login-card').withTags(['wcag2a','wcag2aa','wcag21aa']).analyze()).violations).toEqual([]);
  await expect(page.getByRole('alert')).toHaveText('Wrong username or password.');
  await expectFullyInViewport(page,page.locator('.login-card'));
});

test('first account keeps failed workspace setup recoverable without recreating root',async({page},info)=>{
  await guest(page);await page.setViewportSize({width:375,height:667});
  await page.route('**/api/v1/meta',r=>r.fulfill({json:{...meta,needs_onboarding:true}}));
  let roots=0;let workspaces=0;
  await page.route('**/api/v1/auth/me',r=>r.fulfill({json:{user,real_user:user}}));
  await page.route('**/api/v1/onboarding/root',r=>{roots++;return r.fulfill({json:{token:'fixture-root',user}});});
  await page.route('**/api/v1/auth/capabilities',r=>r.fulfill({json:{capabilities:{}}}));
  await page.route('**/api/v1/workspaces',r=>r.request().method()==='POST'?(++workspaces===1?r.fulfill({status:503,json:{code:'upstream',message:'Workspace unavailable; retry setup'}}):r.fulfill({json:{id:'fixture-ws',name:'Fixture',root_path:'/tmp/fixture'}})):r.fulfill({json:[]}));
  await page.goto('/#/home');await page.getByRole('button',{name:'Get Started'}).click();await page.getByLabel('Password',{exact:true}).fill('Fixture-password-123');await page.getByLabel('Confirm password').fill('Fixture-password-123');await page.getByRole('button',{name:'Continue',exact:true}).click();
  await page.getByLabel('Name',{exact:true}).fill('Fixture');await page.getByLabel('Directory',{exact:true}).fill('/tmp/fixture');await page.getByRole('button',{name:'Continue',exact:true}).click();await page.getByRole('button',{name:'Continue',exact:true}).click();
  await page.getByRole('button',{name:'Finish Setup'}).click();await expect(page.getByText('Workspace unavailable; retry setup')).toBeVisible();
  await page.screenshot({path:info.outputPath('first-account-error.png')});
  await page.getByRole('button',{name:'Finish Setup'}).click();await expect(page.locator('.shell')).toBeVisible();expect(roots).toBe(1);expect(workspaces).toBe(2);
});

for(const v of [{theme:'native',scheme:'light',width:375,height:667},{theme:'native',scheme:'dark',width:1000,height:600},{theme:'warm',scheme:'light',width:375,height:480},{theme:'warm',scheme:'dark',width:834,height:768},{theme:'pro-dark',scheme:'dark',width:1024,height:768}]) test(`guest OTP recovery ${v.theme}-${v.scheme}`,async({page},info)=>{
  await guest(page);await page.setViewportSize(v);await page.emulateMedia({reducedMotion:'reduce'});
  await page.addInitScript(v=>{localStorage.setItem('otto_theme',v.theme);localStorage.setItem('otto_scheme',v.scheme);localStorage.setItem('otto_direction','rtl');},v);
  let verified=false;let verifyCalls=0;
  await page.route('**/api/v1/sessions/otp',r=>r.fulfill(verified?{json:session('otp')}:{status:403,json:{code:'forbidden',message:'share requires email-OTP verification'}}));
  await page.route('**/api/v1/share/whoami',r=>r.fulfill({status:503,json:{code:'upstream',message:'Synthetic role outage'}}));
  await page.route('**/api/v1/share/verify',r=>{verifyCalls++;if(verifyCalls===1)return r.fulfill({status:429,json:{code:'rate_limited',message:'Synthetic throttle'}});verified=true;return r.fulfill({json:{verified:true}});});
  await page.route('**/api/v1/share/extend',r=>r.fulfill({json:{sent:true}}));
  await page.goto('/#/s/otp/token-otp');await page.getByRole('button',{name:'Verify',exact:true}).click();await expect(page.getByRole('alert')).toContainText('6-digit');
  await page.getByLabel('One-time access code').fill('123456');await page.getByRole('button',{name:'Verify',exact:true}).click();await expect(page.getByRole('alert')).toContainText('Too many attempts');
  await page.screenshot({path:info.outputPath('otp-error.png')});
  expect((await new AxeBuilder({page}).include('.error-card').withTags(['wcag2a','wcag2aa','wcag21aa']).analyze()).violations).toEqual([]);
  for(const el of await page.locator('.error-card p,.error-card button,.error-card input').all())expect(await el.evaluate(el=>parseFloat(getComputedStyle(el).fontSize))).toBeGreaterThanOrEqual(11);
  await expectFullyInViewport(page,page.locator('.error-card'));
  await page.getByRole('button',{name:'Re-send code'}).click();await expect(page.getByText('A fresh code has been sent.')).toBeVisible();
  await page.getByLabel('One-time access code').fill('123456');await page.getByRole('button',{name:'Verify',exact:true}).click();await expect(page.locator('.session-title')).toHaveText('Synthetic otp');await expect(page.getByText('You can type',{exact:true})).toBeHidden();
  await expect(page.locator('.xterm-rows')).toContainText('Synthetic guest terminal');
  await page.screenshot({path:info.outputPath('guest-viewer.png')});
});

test('first account long tool versions stay readable on short phone',async({page},info)=>{
  await guest(page);await page.setViewportSize({width:375,height:400});await page.addInitScript(()=>localStorage.setItem('otto_direction','rtl'));
  await page.route('**/api/v1/meta',r=>r.fulfill({json:{...meta,needs_onboarding:true,tools:['claude','codex','clickhouse'].map(name=>({name,found:true,version:`${name}-long-synthetic-build-20260925-with-complete-version-information`}))}}));
  await page.goto('/#/home');await page.getByRole('button',{name:'Get Started'}).click();
  await page.getByLabel('Password',{exact:true}).fill('Fixture-password-123');await page.getByLabel('Confirm password').fill('Fixture-password-123');await page.getByRole('button',{name:'Continue',exact:true}).click();await page.getByRole('button',{name:'Skip',exact:true}).click();await page.getByRole('button',{name:'Continue',exact:true}).click();
  await page.screenshot({path:info.outputPath('first-account-tools.png')});
  expect(await page.locator('.ob-wrap').evaluate(el=>el.scrollWidth-el.clientWidth)).toBeLessThanOrEqual(1);
  for(const row of await page.locator('.tool-row').all()) {
    expect(await row.evaluate(el=>el.scrollWidth-el.clientWidth)).toBeLessThanOrEqual(1);
    expect(await row.evaluate(el=>el.scrollHeight-el.clientHeight), 'Version text must not overlap adjacent rows').toBeLessThanOrEqual(1);
  }
  await page.getByRole('button',{name:'Back',exact:true}).scrollIntoViewIfNeeded();await expectFullyInViewport(page,page.getByRole('button',{name:'Back',exact:true}));
});

test('replacement bearer for the same guest session reloads its permission',async({page})=>{
  await guest(page);
  await page.route('**/api/v1/sessions/same',r=>r.fulfill({json:session('same')}));
  await page.route('**/api/v1/share/whoami',r=>r.fulfill({json:{session_id:'same',role:r.request().headers().authorization==='Bearer token-editor'?'editor':'viewer'}}));
  await page.goto('/#/s/same/token-editor');await expect(page.getByText('You can type',{exact:true})).toBeVisible();
  await page.evaluate(()=>{location.hash='#/s/same/token-viewer';});
  await expect(page.getByText('You can type',{exact:true})).toBeHidden();
});

test('late guest resend and role replies cannot alter a new link',async({page})=>{
  await guest(page);const role=deferredRoute();const extend=deferredRoute();
  await page.route('**/api/v1/sessions/old',r=>r.fulfill({json:session('old')}));
  await page.route('**/api/v1/sessions/otp',r=>r.fulfill({status:403,json:{code:'forbidden',message:'share requires email-OTP verification'}}));
  await page.route('**/api/v1/sessions/new',r=>r.fulfill({json:session('new')}));
  await page.route('**/api/v1/share/whoami',r=>r.request().headers().authorization==='Bearer token-old'?role.hold(r):r.fulfill({json:{session_id:'new',role:'viewer'}}));
  await page.route('**/api/v1/share/extend',extend.hold);
  await page.goto('/#/s/old/token-old');const pendingRole=await role.promise;await navigate(page,'otp');await page.getByRole('button',{name:'Re-send code'}).click();const pendingExtend=await extend.promise;
  await expect(page.getByRole('button',{name:'Verify',exact:true})).toBeDisabled();await navigate(page,'new');await expect(page.locator('.session-title')).toHaveText('Synthetic new');
  await pendingExtend.fulfill({json:{sent:true}});await pendingRole.fulfill({json:{session_id:'old',role:'editor'}});
  await expect(page.locator('.session-title')).toHaveText('Synthetic new');await expect(page.getByText('You can type',{exact:true})).toBeHidden();
});

test('asynchronous file confirmation restores its visible import trigger',async({page})=>{
  await page.addInitScript(()=>localStorage.setItem('otto_firstrun_dismissed','1'));
  // Cancel the confirmation; no settings import or restore is ever sent.
  await page.route('**/api/v1/settings/import',r=>r.fulfill({status:400,json:{code:'bad_request',message:'Fixture prohibits writes'}}));
  await page.goto('/#/settings/backup');
  const trigger=page.getByRole('button',{name:'Import settings…',exact:true});
  const chooser=page.waitForEvent('filechooser');await trigger.click();
  await (await chooser).setFiles({name:'synthetic-settings.json',mimeType:'application/json',buffer:Buffer.from('{"settings":{},"export_format":1,"excluded_keys":[]}')});
  const dialog=page.getByRole('dialog',{name:'Import settings',exact:true});await expect(dialog).toBeVisible();await page.keyboard.press('Escape');await expect(dialog).toBeHidden();await expect(trigger).toBeFocused();
});

test('guest editor input, viewer protection, ended and invalid-link states',async({page},info)=>{
  await guest(page);const input:string[]=[];let endSession=()=>{};
  await page.route('**/api/v1/sessions/same',r=>r.fulfill({json:session('same')}));
  await page.route('**/api/v1/share/whoami',r=>r.fulfill({json:{session_id:'same',role:r.request().headers().authorization==='Bearer token-editor'?'editor':'viewer'}}));
  await page.routeWebSocket(/\/ws\/term\//,socket=>{
    endSession=()=>socket.send(JSON.stringify({type:'status',status:'exited'}));
    socket.onMessage(raw=>{const message=JSON.parse(String(raw));if(message.type==='input')input.push(Buffer.from(message.data,'base64').toString());if(message.type==='scrollback')socket.send(JSON.stringify({type:'scrollback',epoch:1,data:Buffer.from('Synthetic shared output\r\n').toString('base64')}));});
  });
  await page.goto('/#/s/same/token-editor');await expect(page.getByText('You can type',{exact:true})).toBeVisible();await expect(page.locator('.xterm-rows')).toContainText('Synthetic shared output');
  await page.locator('.xterm-helper-textarea').focus();await page.keyboard.type('hello');await expect.poll(()=>input.join('')).toBe('hello');
  await page.evaluate(()=>{location.hash='#/s/same/token-viewer';});await expect(page.getByText('You can type',{exact:true})).toBeHidden();await expect(page.locator('.xterm-rows')).toContainText('Synthetic shared output');
  await page.locator('.xterm-helper-textarea').focus();await page.keyboard.type('blocked');expect(input.join('')).toBe('hello');
  endSession();await expect(page.getByRole('heading',{name:'This session has ended'})).toBeVisible();await page.screenshot({path:info.outputPath('guest-ended.png')});
  await page.getByRole('button',{name:'Reload',exact:true}).click();await expect(page.getByRole('heading',{name:'This session has ended'})).toBeHidden();
  await page.evaluate(()=>{location.hash='#/s/missing';});await expect(page.getByRole('alert')).toContainText('This link is invalid or has expired');
});

test('login recovers from invalid credentials into the authenticated shell',async({page})=>{
  await guest(page);let attempt=0;
  await page.route('**/api/v1/auth/login',r=>++attempt===1?r.fulfill({status:401,json:{code:'unauthorized',message:'Invalid credentials'}}):r.fulfill({json:{token:'fixture-login',user}}));
  await page.route('**/api/v1/auth/me',r=>r.fulfill({json:{user,real_user:user}}));
  await page.route('**/api/v1/auth/capabilities',r=>r.fulfill({json:{capabilities:{}}}));
  await page.goto('/#/home');await page.getByLabel('Username').fill('fixture');await page.getByLabel('Password',{exact:true}).fill('wrong');await page.getByRole('button',{name:'Sign In',exact:true}).click();await expect(page.getByRole('alert')).toHaveText('Wrong username or password.');
  await page.getByLabel('Password',{exact:true}).fill('correct-fixture-password');await page.getByLabel('Password',{exact:true}).press('Enter');await expect(page.locator('.shell')).toBeVisible();expect(attempt).toBe(2);
});
