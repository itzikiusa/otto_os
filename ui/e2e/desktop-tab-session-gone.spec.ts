import { test, expect, type APIRequestContext, type Page } from '@playwright/test';
import { apiCtx, seedWorkspace } from './seed';

// ─────────────────────────────────────────────────────────────────────────────
// A deleted session never leaves its tab behind (owner report, as root: "we
// deleted a session, but it wasn't removed from the tabs"). Every delete path —
// the tab ×, the sidebar row menu, bulk delete in select mode — and a delete
// that happens OUTSIDE this window (the API, another client) must close the
// tab. The last one leans on the `session_removed` event; a missed event is
// caught by the window-return safety net (lib/stores/tabSweep.ts, unit-tested
// in unit/tabSweep.test.ts).
// ─────────────────────────────────────────────────────────────────────────────

let ctx: APIRequestContext;
let base = '';
let wsId = '';
const TITLES = ['Pirlo', 'Totti', 'Baggio', 'Zola'];
const idByTitle: Record<string, string> = {};

test.beforeEach(async ({ page }, info) => {
  test.skip(info.project.name !== 'desktop-browser', 'desktop-browser project only');
  const c = await apiCtx();
  ctx = c.ctx;
  base = c.base;
  wsId = await seedWorkspace(ctx, base, 'Tabs gone WS');
  for (const title of TITLES) {
    const r = await ctx.post(`${base}/api/v1/workspaces/${wsId}/sessions`, {
      data: { kind: 'agent', provider: 'shell', title, cwd: '/tmp', meta: { origin: 'e2e' } },
    });
    if (!r.ok()) throw new Error(`seed ${title} → ${r.status()} ${await r.text()}`);
    idByTitle[title] = (await r.json()).id as string;
  }
  await page.addInitScript((id) => {
    localStorage.setItem('otto_workspace', id as string);
    localStorage.removeItem('otto_close_tab_pref');
  }, wsId);
  await page.goto('/#/agents');
  await expect(page.getByText('Pirlo').first()).toBeVisible({ timeout: 20_000 });
});

test.afterEach(async () => {
  await ctx?.dispose();
});

const exists = async (id: string): Promise<boolean> => (await ctx.get(`${base}/api/v1/sessions/${id}`)).ok();

async function openTab(page: Page, title: string) {
  await page.locator('.nav-item.nested-item', { hasText: title }).first().click();
  const tab = page.locator('.tab', { hasText: title });
  await expect(tab).toBeVisible({ timeout: 10_000 });
  return tab;
}

test('tab ×: Delete closes the tab and removes the session', async ({ page }) => {
  const tab = await openTab(page, 'Pirlo');
  await tab.locator('.tab-close').click();
  await page.getByRole('button', { name: 'Delete session' }).click();
  await expect(tab).toBeHidden({ timeout: 10_000 });
  await expect.poll(() => exists(idByTitle.Pirlo), { timeout: 15_000 }).toBe(false);
});

test('sidebar row menu: Delete… closes the open tab', async ({ page }) => {
  const tab = await openTab(page, 'Totti');
  await page.locator('.nav-item.nested-item', { hasText: 'Totti' }).first().click({ button: 'right' });
  await page.locator('.ctx-item', { hasText: 'Delete…' }).click();
  await page.getByRole('dialog').getByRole('button', { name: 'Delete', exact: true }).click();
  await expect(tab).toBeHidden({ timeout: 10_000 });
  await expect.poll(() => exists(idByTitle.Totti), { timeout: 15_000 }).toBe(false);
});

test('select mode: bulk delete closes every deleted session’s tab', async ({ page }) => {
  const baggio = await openTab(page, 'Baggio');
  const zola = await openTab(page, 'Zola');
  await page.getByTestId('agents-select-toggle').click();
  await page.getByLabel('Select Baggio').check();
  await page.getByLabel('Select Zola').check();
  await page.getByTestId('agents-delete-selected').click();
  await page.getByRole('dialog').getByRole('button', { name: 'Delete', exact: true }).click();
  await expect(baggio).toBeHidden({ timeout: 10_000 });
  await expect(zola).toBeHidden({ timeout: 10_000 });
  await expect.poll(() => exists(idByTitle.Zola), { timeout: 15_000 }).toBe(false);
});

test('a session deleted through the API while its tab is open: the tab disappears', async ({ page }) => {
  const tab = await openTab(page, 'Pirlo');
  const r = await ctx.delete(`${base}/api/v1/sessions/${idByTitle.Pirlo}`);
  expect(r.ok()).toBeTruthy();
  await expect(tab).toBeHidden({ timeout: 15_000 });
  // A reload never brings it back (the saved tab list and the route drop it).
  await page.reload();
  await expect(page.getByText('Totti').first()).toBeVisible({ timeout: 20_000 });
  await expect(page.locator('.tab', { hasText: 'Pirlo' })).toHaveCount(0);
});

