import { test, expect } from '@playwright/test';
import { openPage } from './helpers';
import { apiCtx, seedWorkspace } from './seed';

test.use({ serviceWorkers: 'block' });

test('Canvas queued writes never inherit a later login or reveal its old draft', async ({ page }) => {
  await openPage(page, 'canvas');
  await page.route('**/canvas/scenes', route => route.fulfill({ json: [] }));
  let release!: () => void;
  const held = new Promise<void>(resolve => { release = resolve; });
  let calls = 0;
  await page.route('**/canvas/scenes/identity-fixture', async route => {
    calls++;
    if (calls === 1) await held;
    await route.fulfill({ status: 503, json: { code: 'upstream', message: 'Synthetic held save' } });
  });
  await page.evaluate(async () => {
    const path = '/src/lib/stores/canvas.svelte.ts';
    const { canvas } = await import(path);
    const doc = (source: string) => ({ type: 'otto-canvas', version: 1, format: 'mermaid', source });
    const writes = [canvas.persistDoc('identity-fixture', doc('First private draft')), canvas.persistDoc('identity-fixture', doc('Second private draft')), canvas.del('identity-fixture')];
    (window as any).__canvasIdentityWrites = Promise.allSettled(writes);
  });
  await expect.poll(() => calls).toBe(1);
  await page.evaluate(async () => {
    const path = '/src/lib/api/client.ts';
    const { setToken } = await import(path);
    setToken('synthetic-different-identity');
  });
  release();
  await page.evaluate(() => (window as any).__canvasIdentityWrites);
  expect(calls, 'Queued old saves and deletes must not use the next identity').toBe(1);
  expect(await page.evaluate(async () => {
    const path = '/src/lib/stores/canvas.svelte.ts';
    const { canvas } = await import(path);
    return canvas.docSaveErrors;
  }), 'Old save results must not appear in the next identity').toEqual({});
});

test('Canvas logout invalidates a mounted editor debounce without flushing as the next user', async ({ page }) => {
  // Phones intentionally expose the read-only Canvas preview. Use the supported
  // tablet editor in the mobile WebKit project for this edit/debounce case.
  if ((page.viewportSize()?.width ?? 1280) <= 640) await page.setViewportSize({ width: 834, height: 1112 });
  const { ctx, base } = await apiCtx();
  const workspace = await seedWorkspace(ctx, base);
  const response = await ctx.post(`${base}/api/v1/workspaces/${workspace}/canvas/scenes`, {
    data: { title: 'Private pending editor', doc: { type: 'otto-canvas', version: 1, format: 'mermaid', source: 'flowchart LR\n A[Private] --> B[Draft]' } },
  });
  expect(response.ok()).toBeTruthy();
  const scene = await response.json();
  await ctx.dispose();
  await page.addInitScript(id => localStorage.setItem('otto_workspace', id), workspace);
  await openPage(page, 'canvas');
  await page.locator('.scene-list .row', { hasText: 'Private pending editor' }).getByRole('button').first().click();
  await page.getByTitle('Edit the Mermaid source', { exact: true }).click();
  let writes = 0;
  await page.route(`**/canvas/scenes/${scene.id}`, route => {
    if (route.request().method() !== 'PUT') return route.continue();
    writes++;
    return route.fulfill({ json: {} });
  });
  await page.locator('.cm-content').fill('flowchart LR\n A[Pending private text] --> B[Draft]');
  await page.evaluate(async () => {
    const path = '/src/lib/api/client.ts';
    const { setToken } = await import(path);
    setToken(null);
  });
  await expect(page.locator('.board')).toHaveCount(0);
  await page.waitForTimeout(750); // Beyond the editor's 500ms autosave debounce.
  expect(writes).toBe(0);
  expect(await page.evaluate(async () => {
    const path = '/src/lib/stores/canvas.svelte.ts';
    const { canvas } = await import(path);
    return { id: canvas.currentId, source: canvas.source, errors: canvas.docSaveErrors, scenes: canvas.scenes.length };
  })).toEqual({ id: null, source: null, errors: {}, scenes: 0 });
});
