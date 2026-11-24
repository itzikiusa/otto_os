import { test, expect, type APIRequestContext } from '@playwright/test';
import { apiCtx, seedWorkspace } from './seed';
import { expectFingerSized } from './helpers';

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

// The terminal's ‹ steps back too: the system Back from the list leaves Agents.
test('a terminal\'s ‹ steps back to the list; the system Back then leaves Agents', async ({ page }) => {
  await page.goto('/#/settings');
  await page.evaluate(() => (location.hash = '#/agents'));
  const row = page.locator(`.ai-row[data-workspace="${wsId}"] .tr-main`, { hasText: 'Gullit' });
  await expect(row).toBeVisible({ timeout: 20_000 });
  await row.tap();
  await expect(page.locator('.term-host').first()).toBeVisible({ timeout: 30_000 });
  await page.getByTestId('agents-back-to-list').tap();
  await expect(page.getByTestId('agents-inbox')).toBeVisible();
  await expect.poll(() => page.evaluate(() => location.hash)).toBe('#/agents');
  await page.goBack();
  await expect.poll(() => page.evaluate(() => location.hash)).toMatch(/^#\/settings/);
});

// Phone targets are 44 px: the list's Select / Sort, its New session, and the
// app bar's controls (design review H8).
test('the list and the app bar give 44 px targets', async ({ page }) => {
  await page.goto('/#/agents');
  await expect(page.getByTestId('agents-inbox')).toBeVisible({ timeout: 20_000 });
  // …and the workspace switch and the usage meter (design review 2, H8).
  const targets = [page.getByTestId('agents-select-toggle'), page.getByTestId('agents-sort-toggle'), page.getByTestId('agents-inbox-new'), page.getByTestId('agents-current-ws')];
  await expect(page.getByTestId('threads-open-usage')).toBeVisible({ timeout: 15_000 });
  targets.push(page.getByTestId('threads-open-usage'));
  for (const b of await page.locator('.mtopbar button:visible').all()) targets.push(b);
  for (const t of targets) {
    const box = (await t.boundingBox())!;
    expect(Math.round(box.height), await t.getAttribute('aria-label') ?? await t.innerText()).toBeGreaterThanOrEqual(44);
    expect(Math.round(box.width), await t.getAttribute('aria-label') ?? await t.innerText()).toBeGreaterThanOrEqual(44);
  }
});

// New session on a phone: the Terminal | Chat switch and each card's − / +
// take a finger (design review 2, H8).
test('the New session sheet gives 44 px targets', async ({ page }) => {
  await page.goto('/#/agents');
  await expect(page.getByTestId('agents-inbox')).toBeVisible({ timeout: 20_000 });
  await page.getByTestId('agents-inbox-new').tap();
  const dialog = page.getByRole('dialog', { name: 'New session' });
  const kinds = dialog.getByTestId('ns-kind').getByRole('radio');
  await expect(kinds.first()).toBeVisible({ timeout: 10_000 });
  await expectFingerSized(...(await kinds.all()), ...(await dialog.locator('.count-ctl:visible .cbtn').all()));
});
