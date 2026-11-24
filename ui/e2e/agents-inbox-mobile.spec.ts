import { test, expect, type APIRequestContext } from '@playwright/test';
import { apiCtx, seedWorkspace } from './seed';

// Phone (spec A1 + A6): #/agents is the inbox; a tap pushes the session
// full-height; "Sessions" (or the system Back) returns to the list.

let ctx: APIRequestContext;
let base = '';
let wsId = '';
const SHOTS = process.env.OTTO_E2E_SHOTS;

test.beforeEach(async ({ page }, info) => {
  test.skip(!['iphone-portrait', 'android-phone'].includes(info.project.name), 'phone projects');
  ({ ctx, base } = await apiCtx());
  wsId = await seedWorkspace(ctx, base);
  for (const title of ['Gullit', 'Rijkaard']) {
    const r = await ctx.post(`${base}/api/v1/workspaces/${wsId}/sessions`, {
      data: { kind: 'agent', provider: 'shell', title, cwd: '/tmp', meta: { origin: 'e2e' } },
    });
    if (!r.ok()) throw new Error(`seed ${title} → ${r.status()} ${await r.text()}`);
  }
  await page.addInitScript((id) => {
    localStorage.setItem('otto_workspace', id as string);
    localStorage.setItem('otto_firstrun_dismissed', '1');
    localStorage.setItem('otto_nav_all_ws', '0');
  }, wsId);
});

test.afterEach(async () => {
  await ctx?.dispose();
});

test('the list is the page; a tap pushes the terminal; Back returns', async ({ page }, info) => {
  await page.goto('/#/agents');
  const row = page.locator(`.ai-row[data-workspace="${wsId}"] .tr-main`, { hasText: 'Gullit' });
  await expect(row).toBeVisible({ timeout: 20_000 });
  const box = await row.boundingBox();
  expect(box!.height).toBeGreaterThanOrEqual(44);
  if (SHOTS) await page.screenshot({ path: `${SHOTS}/agents-inbox-${info.project.name}.png` });

  await row.tap();
  await expect(page.locator('.term-host').first()).toBeVisible({ timeout: 30_000 });
  await expect(page.getByTestId('agents-inbox')).toHaveCount(0);
  if (SHOTS) await page.screenshot({ path: `${SHOTS}/agents-session-${info.project.name}.png` });

  await page.getByTestId('agents-back-to-list').tap();
  await expect(page.getByTestId('agents-inbox')).toBeVisible();
  await row.tap();
  await expect(page.locator('.term-host').first()).toBeVisible({ timeout: 30_000 });
  await page.goBack();
  await expect(page.getByTestId('agents-inbox')).toBeVisible();
});
