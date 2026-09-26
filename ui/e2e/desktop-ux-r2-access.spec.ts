import {test, expect} from '@playwright/test';
import AxeBuilder from '@axe-core/playwright';
import {apiCtx, seedWorkspace} from './seed';
import {openApiEditor, expectFullyInViewport} from './helpers';
test.use({serviceWorkers:'block'});

test('API response editor initializes browser LSP and renders server diagnostics', async ({page}, info) => {
  const {ctx, base} = await apiCtx();
  const id = await seedWorkspace(ctx, base); await ctx.dispose();
  await page.addInitScript(id => {localStorage.setItem('otto_workspace',id);localStorage.setItem('otto_firstrun_dismissed','1');}, id);
  const messages: {method:string;params:any}[] = [];
  const warnings:string[] = [];
  page.on('console',m => {if(m.text().includes('externalized')) warnings.push(m.text());});
  await page.route('**/lsp/capabilities', r => r.fulfill({json:{servers:[{lang:'json',available:true}]}}));
  await page.routeWebSocket(/\/ws\/lsp\?/, socket => {
    socket.onMessage(raw => {
      const msg = JSON.parse(String(raw)); messages.push(msg);
      if(msg.method === 'initialize') socket.send(JSON.stringify({jsonrpc:'2.0',id:msg.id,result:{capabilities:{textDocumentSync:1,hoverProvider:true}}}));
      if(msg.method === 'textDocument/didOpen') socket.send(JSON.stringify({jsonrpc:'2.0',method:'textDocument/publishDiagnostics',params:{uri:msg.params.textDocument.uri,diagnostics:[{range:{start:{line:0,character:0},end:{line:0,character:1}},severity:2,message:'Fixture JSON diagnostic'}]}}));
      if(msg.method === 'textDocument/hover') socket.send(JSON.stringify({jsonrpc:'2.0',id:msg.id,result:{contents:{kind:'plaintext',value:'Fixture JSON hover'}}}));
    });
  });
  await page.route('**/api-client/execute',r => r.fulfill({json:{status:200,status_text:'OK',headers:[{key:'Content-Type',value:'application/json'}],body:'{"answer":42}',body_base64:'',truncated:false,too_large:false,duration_ms:7,size_bytes:13,content_type:'application/json',trace:[]}}));
  await openApiEditor(page);
  await page.getByLabel('Request URL',{exact:true}).fill('https://fixture.invalid/lsp');
  await page.getByRole('button',{name:'Send',exact:true}).click();
  await expect(page.locator('.cm-content')).toContainText('answer');
  await expect.poll(() => messages.map(m=>m.method)).toContain('textDocument/didOpen');
  expect(messages.find(m=>m.method === 'textDocument/didOpen')?.params.textDocument).toMatchObject({languageId:'json',text:'{\n  "answer": 42\n}',uri:'file://response.json'});
  await expect(page.locator('.cm-lintRange-warning')).toBeVisible();
  expect(warnings).toEqual([]);
  await page.screenshot({animations:'disabled',path:info.outputPath('lsp-response.png')});
});

test('pointer-opened nested sheets return focus to Browse after Escape',async ({page}) => {
  const {ctx,base}=await apiCtx(); const id=await seedWorkspace(ctx,base); await ctx.dispose();
  await page.addInitScript(id=>{localStorage.setItem('otto_workspace',id);localStorage.setItem('otto_firstrun_dismissed','1');},id);
  await page.goto('/#/agents'); await expect(page.locator('.shell')).toBeVisible();
  await page.keyboard.press('Meta+t');
  const session=page.getByRole('dialog',{name:'New session',exact:true});
  const browse=session.getByRole('button',{name:'Browse…'}).first();
  await browse.click();
  const picker=page.getByRole('dialog',{name:'Choose working directory'});
  await expect(picker).toBeVisible(); await expectFullyInViewport(page,picker);
  await page.keyboard.press('Escape');
  await expect(picker).toBeHidden(); await expect(browse).toBeFocused();
});

const variants = [
  {name:'native-light-compact',theme:'native',scheme:'light',direction:'ltr',width:1100,height:600,zoom:1.2},
  {name:'native-dark-short',theme:'native',scheme:'dark',direction:'ltr',width:1000,height:500,zoom:1},
  {name:'warm-light-phone',theme:'warm',scheme:'light',direction:'rtl',width:375,height:667,zoom:1},
  {name:'warm-dark',theme:'warm',scheme:'dark',direction:'ltr',width:1440,height:900,zoom:1},
  {name:'pro-dark-tablet',theme:'pro-dark',scheme:'dark',direction:'rtl',width:1024,height:768,zoom:1.2},
];
for(const variant of variants) test(`Long Markdown and short sheet: ${variant.name}`,async ({page},info)=>{
  const {ctx,base}=await apiCtx(); const id=await seedWorkspace(ctx,base);
  const name=`access-long-${Date.now()}`;
  const body='---\ndescription: Synthetic shared style fixture\n---\n\n# Long content review\n\nA complete command: `'+ 'long_path_segment/'.repeat(25) +'file.json`\n\n```sh\n./run --description "'+ 'Long code line '.repeat(40)+'"\n```\n\n| First column | Second column | Third column | Final column |\n| --- | --- | --- | --- |\n| '+ 'Long cell content '.repeat(25)+' | Secondary details | More details | LAST-COLUMN |\n\n> Readable shared quotation.\n';
  const response=await ctx.post(`${base}/api/v1/library/skills`,{data:{name,category:'review',description:'Synthetic shared style fixture',body}}); expect(response.ok()).toBeTruthy(); await ctx.dispose();
  // Model the reduced layout space at 120%; native WKWebView zoom itself
  // needs a Tauri run (the browser shell deliberately does not apply CSS zoom).
  await page.setViewportSize({width:Math.round(variant.width/variant.zoom),height:Math.round(variant.height/variant.zoom)});
  await page.emulateMedia({reducedMotion:'reduce'});
  await page.addInitScript(({id,v})=>{localStorage.setItem('otto_workspace',id);localStorage.setItem('otto_firstrun_dismissed','1');localStorage.setItem('otto_theme',v.theme);localStorage.setItem('otto_scheme',v.scheme);localStorage.setItem('otto_direction',v.direction);},{id,v:variant});
  await page.goto('/#/skills-eval');
  await page.getByRole('searchbox',{name:'Search skills'}).fill(name);
  await page.getByTestId('skill-row').click();
  const preview=page.getByTestId('skill-preview'); await expect(preview.locator('table')).toBeVisible();
  await preview.scrollIntoViewIfNeeded();
  await expect(preview.locator('pre')).toHaveAttribute('tabindex','0');
  await preview.locator('pre').focus();
  await preview.locator('pre').press('ArrowRight');
  await expect(preview.locator('pre')).toBeFocused();
  // Playwright WebKit does not perform native arrow-key scrolling, even in
  // a standalone overflow div. Chromium verifies the browser scroll action.
  if(info.project.name==='desktop-browser') await expect.poll(()=>preview.locator('pre').evaluate(el=>el.scrollLeft)).toBeGreaterThan(0);
  for(const selector of ['p','pre','table']) {
    expect(await preview.locator(selector).first().evaluate(el=>{const b=el.getBoundingClientRect();const parent=el.closest('.md-body')!.getBoundingClientRect();return b.left>=parent.left-1&&b.right<=parent.right+1;}),`${selector} stays inside Markdown`).toBe(true);
  }
  expect(await preview.evaluate(el=>el.scrollWidth-el.clientWidth),'Markdown content must not force sideways scrolling for prose').toBeLessThanOrEqual(2);
  await page.screenshot({animations:'disabled',path:info.outputPath('long-markdown.png')});
  expect((await new AxeBuilder({page}).include('[data-testid=skill-preview]').withTags(['wcag2a','wcag2aa','wcag21aa']).analyze()).violations).toEqual([]);
  await page.evaluate(async()=>{const modulePath='/src/lib/confirm.svelte.ts';const {confirmer}=await import(modulePath);void confirmer.choose('Long confirmation details. '.repeat(100),{title:'Review synthetic actions',options:[{label:'Archive 123 sessions',value:'archive',kind:'primary'},{label:'Delete 123 sessions',value:'delete',kind:'danger'}]});});
  const sheet=page.getByRole('dialog',{name:'Review synthetic actions'}); await expect(sheet).toBeVisible();
  for(const button of await sheet.locator('footer button').all()) await expectFullyInViewport(page,button);
  await expect(sheet.locator('.sheet-body')).toHaveAttribute('tabindex','0');
  await sheet.locator('.sheet-body').focus();
  await page.keyboard.press('ArrowDown');
  await expect(sheet.locator('.sheet-body')).toBeFocused();
  if(info.project.name==='desktop-browser') await expect.poll(()=>sheet.locator('.sheet-body').evaluate(el=>el.scrollTop)).toBeGreaterThan(0);
  await page.screenshot({animations:'disabled',path:info.outputPath('short-sheet.png')});
  expect((await new AxeBuilder({page}).include('.sheet').withTags(['wcag2a','wcag2aa','wcag21aa']).analyze()).violations).toEqual([]);
  await sheet.getByRole('button',{name:'Cancel',exact:true}).click(); await expect(sheet).toBeHidden();
});

test('pointer drawer and nested workspace sheets unwind one layer at a time',async ({page})=>{
  await page.setViewportSize({width:375,height:667});
  await page.addInitScript(()=>localStorage.setItem('otto_firstrun_dismissed','1'));
  await page.goto('/#/home');
  const trigger=page.getByRole('button',{name:'Open navigator',exact:true}); await trigger.click();
  const drawer=page.getByRole('dialog',{name:'Navigator',exact:true});
  const add=drawer.getByRole('button',{name:'Add workspace',exact:true}); await add.click();
  const sheet=page.getByRole('dialog',{name:'Add Workspace',exact:true}); await expect(sheet).toBeVisible();
  const browse=sheet.getByRole('button',{name:/Browse/}).first(); await browse.click();
  const picker=page.getByRole('dialog',{name:'Choose project directory'}); await expect(picker).toBeVisible();
  await page.keyboard.press('Escape'); await expect(picker).toBeHidden(); await expect(browse).toBeFocused(); await expect(sheet).toBeVisible();
  await page.keyboard.press('Escape'); await expect(sheet).toBeHidden(); await expect(add).toBeFocused(); await expect(drawer).toBeVisible();
  await page.keyboard.press('Escape'); await expect(drawer).toBeHidden(); await expect(trigger).toBeFocused();
});
