import { test, expect, type APIRequestContext } from '@playwright/test';
import { apiCtx, seedWorkspace } from './seed';

// ─────────────────────────────────────────────────────────────────────────────
// Closing a session tab ENDS the session (desktop-browser only).
//
// Regression: the tab × used to offer "Close tab (keeps running)" and, worse,
// skipped the dialog entirely when the client's cached `live` flag was stale —
// so a session silently stayed alive behind a closed tab. Now every close
// gesture (tab ×, sidebar ×) asks Archive / Delete (Cancel keeps the tab),
// and a remembered choice applies without asking.
// ─────────────────────────────────────────────────────────────────────────────

let ctx: APIRequestContext;
let base = '';
let wsId = '';
const TITLES = ['Kaka', 'Nesta', 'Maldini', 'Gattuso'];
const idByTitle: Record<string, string> = {};

async function seedSessions(): Promise<void> {
  wsId = await seedWorkspace(ctx, base);
  for (const title of TITLES) {
    const r = await ctx.post(`${base}/api/v1/workspaces/${wsId}/sessions`, {
      data: { kind: 'agent', provider: 'shell', title, cwd: '/tmp', meta: { origin: 'e2e' } },
    });
    if (!r.ok()) throw new Error(`seed ${title} → ${r.status()} ${await r.text()}`);
    idByTitle[title] = (await r.json()).id as string;
  }
}

async function isArchived(id: string): Promise<boolean> {
  const r = await ctx.get(`${base}/api/v1/sessions/${id}`);
  if (!r.ok()) return false;
  return ((await r.json()) as { archived?: boolean }).archived === true;
}

async function exists(id: string): Promise<boolean> {
  return (await ctx.get(`${base}/api/v1/sessions/${id}`)).ok();
}

test.beforeEach(async ({ page }, info) => {
  test.skip(info.project.name !== 'desktop-browser', 'desktop-browser project only');
  const c = await apiCtx();
  ctx = c.ctx;
  base = c.base;
  await seedSessions();
  await page.addInitScript((id) => {
    localStorage.setItem('otto_workspace', id as string);
    localStorage.removeItem('otto_close_tab_pref');
  }, wsId);
  await page.goto('/#/agents');
  await expect(page.getByText('Kaka').first()).toBeVisible({ timeout: 20_000 });
});

test.afterEach(async () => {
  await ctx?.dispose();
});

/** Open a session from the sidebar and return its tab-bar tab. */
async function openTab(page: import('@playwright/test').Page, title: string) {
  await page.locator('.nav-item.nested-item', { hasText: title }).first().click();
  const tab = page.locator('.tab', { hasText: title });
  await expect(tab).toBeVisible({ timeout: 10_000 });
  return tab;
}

test('tab ×: Cancel keeps the session; Archive ends it and closes the tab', async ({ page }) => {
  const tab = await openTab(page, 'Kaka');
  await tab.locator('.tab-close').click();
  const dialog = page.locator('.cf-msg');
  await expect(dialog).toContainText('ends the session');
  await page.getByRole('button', { name: 'Cancel' }).click();
  await expect(dialog).toBeHidden();
  await expect(tab).toBeVisible();
  expect(await isArchived(idByTitle.Kaka)).toBe(false);

  await tab.locator('.tab-close').click();
  await page.getByRole('button', { name: 'Archive session' }).click();
  await expect(tab).toBeHidden({ timeout: 10_000 });
  await expect.poll(() => isArchived(idByTitle.Kaka), { timeout: 15_000 }).toBe(true);
  // Untouched neighbours.
  expect(await isArchived(idByTitle.Nesta)).toBe(false);
});

test('tab ×: Delete removes the session', async ({ page }) => {
  const tab = await openTab(page, 'Nesta');
  await tab.locator('.tab-close').click();
  await page.getByRole('button', { name: 'Delete session' }).click();
  await expect(tab).toBeHidden({ timeout: 10_000 });
  await expect.poll(() => exists(idByTitle.Nesta), { timeout: 15_000 }).toBe(false);
});

test('remembered choice applies to later closes without a dialog (tab × and sidebar ×)', async ({ page }) => {
  const tab = await openTab(page, 'Maldini');
  await tab.locator('.tab-close').click();
  await page.locator('.cf-remember input[type=checkbox]').check();
  await page.getByRole('button', { name: 'Archive session' }).click();
  await expect(tab).toBeHidden({ timeout: 10_000 });
  await expect.poll(() => isArchived(idByTitle.Maldini), { timeout: 15_000 }).toBe(true);
  expect(await page.evaluate(() => localStorage.getItem('otto_close_tab_pref'))).toBe('archive');

  // Sidebar × on another session: no dialog, archived straight away.
  const row = page.locator('.nav-item.nested-item', { hasText: 'Gattuso' }).first();
  await row.hover();
  await page.locator('.nested-row', { hasText: 'Gattuso' }).first().locator('button[aria-label="Close session"]').click();
  await expect(page.locator('.cf-msg')).toHaveCount(0);
  await expect.poll(() => isArchived(idByTitle.Gattuso), { timeout: 15_000 }).toBe(true);
});

test('archived list: multi-select deletes the checked sessions in one confirm', async ({ page }) => {
  // Archive two sessions via the API so the Archived list has ≥2 rows.
  for (const t of ['Kaka', 'Nesta']) {
    const r = await ctx.post(`${base}/api/v1/sessions/${idByTitle[t]}/archive`);
    if (!r.ok()) throw new Error(`archive ${t} → ${r.status()}`);
  }
  await page.reload();
  await expect(page.getByText('Maldini').first()).toBeVisible({ timeout: 20_000 });
  await page.getByRole('button', { name: /^Archived/ }).click();
  await page.getByLabel('Select Kaka').check();
  await page.getByLabel('Select Nesta').check();
  const del = page.getByTestId('archived-delete-selected');
  await expect(del).toContainText('Delete (2)');
  await del.click();
  await expect(page.locator('.cf-msg')).toContainText('2 archived sessions');
  await page.getByRole('dialog').getByRole('button', { name: 'Delete', exact: true }).click();
  await expect.poll(() => exists(idByTitle.Kaka), { timeout: 15_000 }).toBe(false);
  await expect.poll(() => exists(idByTitle.Nesta), { timeout: 15_000 }).toBe(false);
  expect(await exists(idByTitle.Maldini)).toBe(true);
});

test('"Always delete" pref: split-pane header × ends the session without a dialog', async ({ page }) => {
  // Remember "Delete" through the tab × dialog (the beforeEach init script
  // clears the pref on every load, so it must be set in-page).
  const tab = await openTab(page, 'Kaka');
  await tab.locator('.tab-close').click();
  await page.locator('.cf-remember input[type=checkbox]').check();
  await page.getByRole('button', { name: 'Delete session' }).click();
  await expect(tab).toBeHidden({ timeout: 10_000 });
  await expect.poll(() => exists(idByTitle.Kaka), { timeout: 15_000 }).toBe(false);

  await openTab(page, 'Maldini');
  await openTab(page, 'Gattuso');
  // ⌘D splits the focused session into a second pane → header × shows.
  await page.keyboard.press('Meta+d');
  const hdrClose = page.locator('button[aria-label="Close session (⌘W)"]').first();
  await expect(hdrClose).toBeVisible({ timeout: 10_000 });
  await hdrClose.click();
  await expect(page.locator('.cf-msg')).toHaveCount(0);
  // Exactly one of the two panes' sessions is gone; the other survives.
  await expect
    .poll(async () => (await exists(idByTitle.Gattuso)) !== (await exists(idByTitle.Maldini)), { timeout: 15_000 })
    .toBe(true);
});

test('agents list: select mode archives / deletes the checked sessions in bulk', async ({ page }) => {
  await page.getByTestId('agents-select-toggle').click();
  await page.getByLabel('Select Kaka').check();
  await page.getByLabel('Select Nesta').check();
  await page.getByTestId('agents-archive-selected').click();
  await expect.poll(() => isArchived(idByTitle.Kaka), { timeout: 15_000 }).toBe(true);
  await expect.poll(() => isArchived(idByTitle.Nesta), { timeout: 15_000 }).toBe(true);
  expect(await isArchived(idByTitle.Maldini)).toBe(false);
  // Select mode exits after the batch.
  await expect(page.getByTestId('agents-select-tools')).toHaveCount(0);

  await page.getByTestId('agents-select-toggle').click();
  await page.getByLabel('Select Maldini').check();
  await page.getByTestId('agents-delete-selected').click();
  await expect(page.locator('.cf-msg')).toContainText('Delete 1 session');
  await page.getByRole('dialog').getByRole('button', { name: 'Delete', exact: true }).click();
  await expect.poll(() => exists(idByTitle.Maldini), { timeout: 15_000 }).toBe(false);
  expect(await exists(idByTitle.Gattuso)).toBe(true);
});
