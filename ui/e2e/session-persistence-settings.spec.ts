import { test, expect, type APIRequestContext } from '@playwright/test';
import { apiCtx, seedWorkspace } from './seed';

// Session lifetime settings — E2E against the isolated test daemon. The Daemon
// settings page's Sessions group shows `session_persistence` (default ON) and
// the manual idle-suspend grace (default 24 h), and Save writes exactly the
// keys that changed. Daemon state is global → pin the file to one device
// project; always restore the defaults.

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
  await ctx.put(api('/settings'), {
    data: { session_persistence: true, manual_idle_suspend_secs: 86400 },
  });
  await ctx.dispose();
});

test('Daemon settings shows the session lifetime defaults and saves changes', async ({ page }) => {
  await page.addInitScript((w) => localStorage.setItem('otto_workspace', w as string), ws);
  await page.goto('/#/settings/daemon');
  const persist = page.getByTestId('session-persistence');
  const grace = page.getByTestId('manual-idle-grace');
  await expect(persist).toBeChecked();
  await expect(grace).toHaveValue('86400');

  await grace.selectOption('14400'); // 4 h
  await persist.uncheck();
  await page.getByRole('button', { name: 'Save', exact: true }).click();

  await expect
    .poll(async () => {
      const s = await (await ctx.get(api('/settings'))).json();
      return [s.session_persistence, s.manual_idle_suspend_secs];
    })
    .toEqual([false, 14400]);
});
