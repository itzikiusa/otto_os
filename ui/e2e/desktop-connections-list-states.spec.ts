import { test, expect } from '@playwright/test';
import { apiCtx, seedWorkspace } from './seed';
import { mockDbRoutes, seedMockDbConnection } from './db-mock';
import { pickMenuItem } from './helpers';

// Connections list states the review found wrong: a pending Delete survives a
// page change, parent sections don't claim to be empty, a failed Kafka read
// says so with Retry, the hover ⋯ never covers line 3's text, and a slow deep
// link can't leave the workbench's URL behind. Desktop browser only.
test.use({ serviceWorkers: 'block' });

let wsId = '';
test.beforeAll(async () => {
  const { ctx, base } = await apiCtx();
  wsId = await seedWorkspace(ctx, base);
  await ctx.dispose();
});
test.beforeEach(async ({ page }, info) => {
  test.skip(info.project.name !== 'desktop-browser', 'desktop-browser only');
  await page.addInitScript((w) => {
    localStorage.setItem('otto_workspace', w as string);
    localStorage.setItem('otto_connhub_filter', 'all');
  }, wsId);
});

async function seedSsh(name: string, host = 'undo.invalid', section_id: string | null = null): Promise<string> {
  const { ctx, base } = await apiCtx();
  const res = await ctx.post(`${base}/api/v1/workspaces/${wsId}/connections`, { data: { name, kind: 'ssh', params: { host }, secret: null, section_id } });
  expect(res.ok(), await res.text()).toBeTruthy();
  const { id } = await res.json();
  await ctx.dispose();
  return id as string;
}

test('a row deleted with Undo stays hidden after leaving the page and coming back (R1-006)', async ({ page }) => {
  const name = `away-${Date.now()}`;
  await seedSsh(name);
  await page.goto('/#/connections');
  const row = page.locator('.conn-row', { hasText: name });
  await pickMenuItem(page, () => row.click({ button: 'right' }), 'Delete');
  await expect(row).toHaveCount(0);
  await page.locator('.toast', { hasText: name }).hover(); // holds the Undo window open
  await page.evaluate(() => (location.hash = '#/git'));
  await expect(page.locator('.conn-root')).toHaveCount(0); // the list page is unmounted
  await page.evaluate(() => (location.hash = '#/connections'));
  await expect(page.locator('.conn-row').first()).toBeVisible();
  await expect(row).toHaveCount(0);
  await page.getByRole('button', { name: 'Undo', exact: true }).click();
  await expect(row).toBeVisible();
});

test('a section holding only sub-sections does not say it is empty (R1-091)', async ({ page }) => {
  const { ctx, base } = await apiCtx();
  const tag = `P${Date.now()}`;
  const post = async (data: object) => (await (await ctx.post(`${base}/api/v1/workspaces/${wsId}/connection-sections`, { data })).json()).id as string;
  const parent = await post({ name: tag });
  const child = await post({ name: 'Child', parent_id: parent });
  await ctx.dispose();
  await seedSsh(`${tag}-row`, 'child.invalid', child);
  await page.goto('/#/connections');
  await expect(page.locator('.conn-row', { hasText: `${tag}-row` })).toBeVisible();
  await expect(page.getByRole('region', { name: tag, exact: true })).toBeVisible();
  await expect(page.getByRole('region', { name: tag, exact: true }).getByText('No connections in this section.')).toHaveCount(0);
});

test('a failed Kafka cluster read shows inline with Retry (R1-070)', async ({ page }) => {
  await seedSsh(`kafka-err-${Date.now()}`);
  await page.route(/\/brokers\/clusters$/, (r) => r.fulfill({ status: 500, contentType: 'application/json', body: '{"error":"brokers down"}' }));
  await page.goto('/#/connections');
  const alert = page.getByRole('alert').filter({ hasText: 'Couldn’t load Kafka clusters' });
  await expect(alert).toBeVisible();
  await page.unroute(/\/brokers\/clusters$/);
  await alert.getByRole('button', { name: 'Retry' }).click();
  await expect(alert).toHaveCount(0);
});

test('a slow deep link that settles last does not pin the URL (R1-048)', async ({ page }) => {
  const { ctx, base } = await apiCtx();
  const aName = `deep-a-${Date.now()}`;
  const a = await seedMockDbConnection(ctx, base, wsId, aName);
  const bName = `deep-b-${Date.now()}`;
  const b = await seedMockDbConnection(ctx, base, wsId, bName);
  await ctx.dispose();
  await mockDbRoutes(page, a);
  await mockDbRoutes(page, b);
  await page.route(new RegExp(`/connections/${a}/db/`), async (r) => {
    await new Promise((res) => setTimeout(res, 3000));
    await r.fallback();
  });
  await page.goto(`/#/database/${a}`);
  await expect(page.getByRole('heading', { level: 1, name: aName })).toBeVisible(); // A's open is in flight (delayed 3 s)
  await page.evaluate((id) => (location.hash = `#/database/${id}`), b);
  await expect(page.locator('.conn-tab', { hasText: bName })).toBeVisible();
  await page.waitForTimeout(3500); // ui-guards: allow — A's slow open settles after B's
  await page.locator('.conn-tab', { hasText: bName }).getByRole('button', { name: 'Close connection tab' }).click();
  await expect(page).toHaveURL(new RegExp(`#/database/${a}$`));
});
