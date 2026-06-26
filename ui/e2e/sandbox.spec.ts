import { test, expect, type APIRequestContext } from '@playwright/test';
import { apiCtx, seedWorkspace } from './seed';

// Process sandbox — E2E against the isolated test daemon. Asserts the opt-in
// `process_sandbox` setting round-trips through the API and the Daemon settings
// page surfaces the toggle + network selector. Daemon state is global → pin the
// whole file to one device project; always restore the disabled default.

test.describe.configure({ mode: 'serial' });
test.beforeEach(({}, testInfo) => {
  test.skip(
    testInfo.project.name !== 'iphone-portrait',
    'settings state is global to the daemon; run on a single project only',
  );
});

let ctx: APIRequestContext;
let base = '';
let ws = '';
const api = (p: string) => `${base}/api/v1${p}`;

test.beforeAll(async () => {
  const c = await apiCtx();
  ctx = c.ctx;
  base = c.base;
  ws = await seedWorkspace(ctx, base);
});

test.afterAll(async () => {
  // Never leave the sandbox enabled for other specs' sessions.
  const s = await (await ctx.get(api('/settings'))).json();
  await ctx.put(api('/settings'), {
    data: { ...s, process_sandbox: { enabled: false, network: 'full' } },
  });
  await ctx.dispose();
});

test('process_sandbox setting round-trips', async () => {
  const before = await (await ctx.get(api('/settings'))).json();
  const put = await ctx.put(api('/settings'), {
    data: { ...before, process_sandbox: { enabled: true, network: 'loopback' } },
  });
  expect(put.ok()).toBeTruthy();

  const after = await (await ctx.get(api('/settings'))).json();
  expect(after.process_sandbox.enabled).toBe(true);
  expect(after.process_sandbox.network).toBe('loopback');

  // Restore disabled immediately so no later session gets confined.
  const off = await ctx.put(api('/settings'), {
    data: { ...after, process_sandbox: { enabled: false, network: 'full' } },
  });
  expect(off.ok()).toBeTruthy();
  const final = await (await ctx.get(api('/settings'))).json();
  expect(final.process_sandbox.enabled).toBe(false);
});

test('Daemon settings exposes the Process Sandbox controls', async ({ page }) => {
  await page.addInitScript((w) => localStorage.setItem('otto_workspace', w as string), ws);
  await page.goto('/#/settings/daemon');
  await expect(page.getByTestId('sandbox-enabled')).toBeVisible();
  await expect(page.getByTestId('sandbox-network')).toBeAttached();
});
