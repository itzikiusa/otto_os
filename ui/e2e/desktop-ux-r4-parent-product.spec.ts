import { test, expect } from '@playwright/test';
import { openPage } from './helpers';

test.use({ serviceWorkers: 'block' });

test('Product queued artifact saves do not inherit a later login', async ({ page }) => {
  await openPage(page, 'home');
  let release!: () => void;
  const held = new Promise<void>(resolve => { release = resolve; });
  let calls = 0;
  await page.route('**/product/attachments/identity-fixture/content', async route => {
    calls++;
    if (calls === 1) await held;
    await route.fulfill({ status: 503, json: { code: 'upstream', message: 'Synthetic held save' } });
  });
  await page.evaluate(async () => {
    const path = '/src/lib/stores/product.svelte.ts';
    const { product } = await import(path);
    void product.saveAttachmentContent('identity-fixture', 'First private draft').catch(() => {});
    const first = product.flushAttachmentContent('identity-fixture');
    void product.saveAttachmentContent('identity-fixture', 'Second private draft').catch(() => {});
    const second = product.flushAttachmentContent('identity-fixture');
    (window as any).__productIdentityWrites = Promise.allSettled([first, second]);
  });
  await expect.poll(() => calls).toBe(1);
  await page.evaluate(async () => {
    const path = '/src/lib/api/client.ts';
    const { setToken } = await import(path);
    setToken('synthetic-different-identity');
  });
  release();
  await page.evaluate(() => (window as any).__productIdentityWrites);
  expect(calls, 'Old queued content must not be sent with the next login').toBe(1);
  expect(await page.evaluate(async () => {
    const path = '/src/lib/stores/product.svelte.ts';
    const { product } = await import(path);
    return { state: product.saveState, unsaved: product.hasUnsavedContent('identity-fixture') };
  })).toEqual({ state: {}, unsaved: false });
});

test('Product logout drops pending artifact debounce before its request', async ({ page }) => {
  await openPage(page, 'home');
  let calls = 0;
  await page.route('**/product/attachments/identity-fixture/content', route => {
    calls++;
    return route.fulfill({ status: 503, json: { code: 'upstream', message: 'Synthetic save' } });
  });
  await page.evaluate(async () => {
    const storePath = '/src/lib/stores/product.svelte.ts';
    const clientPath = '/src/lib/api/client.ts';
    const { product } = await import(storePath);
    const { setToken } = await import(clientPath);
    void product.saveAttachmentContent('identity-fixture', 'Private debounced draft').catch(() => {});
    setToken(null);
  });
  await page.waitForTimeout(850); // Beyond the actual 600ms content debounce.
  expect(calls).toBe(0);
});
