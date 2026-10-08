import { test, expect } from '@playwright/test';
import { createRequire } from 'node:module';
import { apiCtx, seedWorkspace } from './seed';
import { openApiEditor, expectNoHorizontalOverflow } from './helpers';
import { resolve } from 'node:path';
const { WebSocketServer } = createRequire(import.meta.url)('ws');

test.use({ serviceWorkers: 'block' });

test('a WebSocket belongs to its initiating request tab', async ({ page }, info) => {
  test.skip(info.project.name !== 'desktop-browser', 'desktop browser fixture');
  const { ctx, base } = await apiCtx();
  const server = new WebSocketServer({ host: '127.0.0.1', port: 0 });
  await new Promise<void>(ready => server.once('listening', ready));
  const address = server.address();
  if (typeof address === 'string') throw new Error('Expected local fixture port');
  let closed = 0;
  const messages: string[] = [];
  server.on('connection', (socket: {send: (data: string) => void; on: (event: string, fn: (data: Buffer) => void) => void}) => {
    socket.send('Message from request A');
    socket.on('message', data => messages.push(data.toString()));
    socket.on('close', () => closed++);
  });
  try {
    await page.setViewportSize({width: 1440, height: 900});
    const wid = await seedWorkspace(ctx, base);
    expect((await ctx.patch(`${base}/api/v1/workspaces/${wid}`, { data: { settings: { api_client: { allow_local: true } } } })).ok()).toBeTruthy();
    await page.addInitScript(({wid, port}) => {
      localStorage.setItem('otto_workspace', wid);
      localStorage.setItem('otto_scheme', 'light');
      const draft = (tabId: string, name: string) => ({tabId, name, requestId: null, kind: 'websocket', method: 'GET', url: `ws://127.0.0.1:${port}`, headers: [], query: [], body_mode: 'none', body: '', auth: {type: 'none'}});
      localStorage.setItem(`otto_api_tabs_v1:${wid}`, JSON.stringify({tabs: [draft('stream-a', 'Request A'), draft('stream-b', 'Request B')], active: 0}));
    }, {wid, port: address.port});
    await openApiEditor(page);
    await page.getByRole('button', {name: 'Connect', exact: true}).click();
    await expect(page.getByText('Message from request A', {exact: true})).toBeVisible();
    await page.getByLabel('Message to send').fill('sent by A');
    await page.getByRole('button', {name: 'Send message', exact: true}).click();
    await expect.poll(() => messages).toEqual(['sent by A']);
    const evidence = resolve('../docs/reviews/quality-20261008/evidence/R05');
    await page.screenshot({path: `${evidence}/stream-loaded-light.png`});
    await page.locator('.req-tab').nth(1).click();
    await expect(page.getByLabel('Message to send')).toBeDisabled();
    await expect(page.getByText('Message from request A', {exact: true})).toHaveCount(0);
    await expect.poll(() => closed).toBe(1);
    await expect(page.getByRole('button', {name: 'Connect', exact: true})).toBeEnabled();
    await page.evaluate(async () => {
      const modulePath = '/src/lib/stores/ui.svelte.ts';
      const {ui} = await import(/* @vite-ignore */ modulePath);
      ui.setScheme('dark');
    });
    await page.screenshot({path: `${evidence}/stream-empty-dark.png`});
    await page.setViewportSize({width: 390, height: 844});
    await expectNoHorizontalOverflow(page);
    await page.screenshot({path: `${evidence}/stream-empty-phone.png`});
  } finally {
    for (const socket of server.clients) socket.terminate();
    await new Promise<void>(done => server.close(() => done()));
    await ctx.dispose();
  }
});

test('a delayed proto upload cannot replace another request tab', async ({ page }, info) => {
  test.skip(info.project.name !== 'desktop-browser', 'desktop browser fixture');
  const {ctx, base} = await apiCtx();
  try {
    const wid = await seedWorkspace(ctx, base);
    await page.addInitScript(wid => {
      localStorage.setItem('otto_workspace', wid);
      const text = File.prototype.text;
      File.prototype.text = function() {
        if (this.name !== 'late.proto') return text.call(this);
        return new Promise(resolve => {
          (window as Window & {releaseProtoRead?: () => void}).releaseProtoRead = () => resolve('syntax = "proto3"; message A {}');
        });
      };
    }, wid);
    await openApiEditor(page);
    await page.getByLabel('Request type').selectOption('grpc');
    await page.locator('input[type=file][accept=".proto"]').setInputFiles({name:'late.proto',mimeType:'text/plain',buffer:Buffer.from('placeholder')});
    await page.locator('.req-tab-new').click();
    await page.getByLabel('Request URL').fill('https://b.test');
    await page.evaluate(() => (window as Window & {releaseProtoRead?: () => void}).releaseProtoRead?.());
    await expect.poll(() => page.evaluate(async () => {
      const path = '/src/lib/stores/apiClient.svelte.ts';
      const {apiClient} = await import(/* @vite-ignore */ path);
      return apiClient.draft.proto ?? '';
    })).toBe('');
    await expect(page.getByLabel('Request URL')).toHaveValue('https://b.test');
  } finally { await ctx.dispose(); }
});

test('reflection cannot publish services after its endpoint is edited', async ({ page }, info) => {
  test.skip(info.project.name !== 'desktop-browser', 'desktop browser fixture');
  const {ctx, base} = await apiCtx();
  try {
    const wid = await seedWorkspace(ctx, base);
    await page.addInitScript(wid => localStorage.setItem('otto_workspace', wid), wid);
    let release!: () => void;
    const ready = new Promise<void>(resolve => { release = resolve; });
    let started = false;
    await page.route('**/api-client/grpc/reflect', async route => {
      started = true;
      await ready;
      await route.fulfill({json:{services:[{name:'OldServer',methods:[{name:'Get',full:'/OldServer/Get',input_type:'Input',output_type:'Output',input_schema:'{}',client_streaming:false,server_streaming:false}]}]}});
    });
    await openApiEditor(page);
    await page.getByLabel('Request type').selectOption('grpc');
    await page.getByLabel('Request URL').fill('https://a.test');
    await page.getByRole('button', {name:'Load from server'}).click();
    await expect.poll(() => started).toBe(true);
    await page.getByLabel('Request URL').fill('https://b.test');
    release();
    await expect(page.getByRole('button', {name:'Load from server'})).toBeEnabled();
    await expect(page.getByText('OldServer', {exact:true})).toHaveCount(0);
    expect(await page.evaluate(async () => {
      const path = '/src/lib/stores/apiClient.svelte.ts';
      const {apiClient} = await import(/* @vite-ignore */ path);
      return apiClient.draft.grpc_method ?? '';
    })).toBe('');
  } finally { await ctx.dispose(); }
});
