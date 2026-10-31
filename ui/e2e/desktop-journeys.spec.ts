import { test, expect, type APIRequestContext, type Page } from '@playwright/test';
import { apiCtx, seedWorkspace } from './seed';

// ─────────────────────────────────────────────────────────────────────────────
// Behavioural journeys for the S20 UX fixes (S20-306): the iteration-2 fixes
// were pinned only by source-regex unit tests, so a refactor that kept the
// strings but broke the flow (an inverted `{#if}`, a lost button `type`) still
// passed. These drive the real UI against the isolated e2e daemon:
//   • Share: on the loopback-only daemon a minted link warns "only works on
//     this Mac" instead of offering a phone QR; a LAN link (reach "lan") keeps
//     the QR and says it is Wi-Fi-only behind a self-signed certificate.
//   • Scheduled task form: the "Next fires" preview comes from the daemon.
// ─────────────────────────────────────────────────────────────────────────────

test.use({ serviceWorkers: 'block' });

let ctx: APIRequestContext;
let base = '';
let wsId = '';

test.beforeEach(async ({ page }, info) => {
  test.skip(info.project.name !== 'desktop-browser', 'desktop-browser project only');
  const c = await apiCtx();
  ctx = c.ctx;
  base = c.base;
  wsId = await seedWorkspace(ctx, base, `Journeys ${Date.now()}`);
  await page.addInitScript((id) => {
    localStorage.setItem('otto_workspace', id as string);
    localStorage.setItem('otto_rail_expanded', '0');
  }, wsId);
});

test.afterEach(async () => {
  await ctx?.dispose();
});

async function openShareFor(page: Page, title: string): Promise<void> {
  const r = await ctx.post(`${base}/api/v1/workspaces/${wsId}/sessions`, {
    data: { kind: 'agent', provider: 'shell', title, cwd: '/tmp', meta: { origin: 'e2e' } },
  });
  expect(r.ok()).toBeTruthy();
  const id = ((await r.json()) as { id: string }).id;
  await page.goto(`/#/agents/${id}`);
  const tab = page.locator('.tab', { hasText: title });
  await expect(tab).toBeVisible({ timeout: 15_000 });
  await tab.click({ button: 'right' });
  await page.locator('.ctx-item', { hasText: 'Share…' }).click();
  await expect(page.getByRole('button', { name: 'Generate link' })).toBeVisible({ timeout: 10_000 });
}

async function mintViewerLink(page: Page): Promise<void> {
  await page.getByRole('button', { name: 'Generate link' }).click();
  // Every link is outward-facing: the confirm names where it goes first.
  await page.getByRole('dialog').getByRole('button', { name: 'Create viewer link' }).click();
}

test('share: a loopback-only link warns instead of showing a phone QR', async ({ page }) => {
  await openShareFor(page, 'Journey share local');
  // The isolated daemon has no Public link domain and no bound LAN listener,
  // so the REAL mint answers reach "local".
  const minted = page.waitForResponse((r) => /\/sessions\/[^/]+\/share$/.test(r.url()) && r.request().method() === 'POST');
  await mintViewerLink(page);
  const resp = await minted;
  expect(resp.ok()).toBeTruthy();
  const body = (await resp.json()) as { reach?: string; reachable_remotely: boolean };
  expect(body.reachable_remotely).toBe(false);
  expect(body.reach).toBe('local');
  await expect(page.getByTestId('share-local-only')).toBeVisible();
  await expect(page.getByTestId('share-local-only')).toContainText('only works on this Mac');
  await expect(page.locator('canvas.sm-qr')).toHaveCount(0);
  await expect(page.getByTestId('share-lan-only')).toHaveCount(0);
});

test('share: a LAN link keeps the QR and says Wi-Fi only + self-signed certificate', async ({ page }) => {
  await openShareFor(page, 'Journey share lan');
  // A bound LAN listener can't be faked on the shared isolated daemon (it is
  // process-wide): answer the mint as the daemon would with one serving.
  await page.route('**/api/v1/sessions/*/share', async (route) => {
    if (route.request().method() !== 'POST') return route.continue();
    const real = await route.fetch();
    const json = (await real.json()) as Record<string, unknown>;
    const url = String(json.url).replace(/^https?:\/\/[^/]+/, 'https://192.168.1.20:7700');
    await route.fulfill({ json: { ...json, url, reachable_remotely: true, reach: 'lan' } });
  });
  await mintViewerLink(page);
  await expect(page.locator('canvas.sm-qr')).toBeVisible();
  await expect(page.getByTestId('share-lan-only')).toContainText('Works on your Wi-Fi only');
  await expect(page.getByTestId('share-lan-only')).toContainText('self-signed certificate');
  await expect(page.getByTestId('share-local-only')).toHaveCount(0);
});

test('scheduled task form: "Next fires" comes from the daemon before Save', async ({ page }) => {
  await page.goto('/#/scheduled-tasks');
  await page.getByRole('button', { name: 'New task' }).first().click();
  await page.getByLabel('Cadence').selectOption('cron');
  await page.getByLabel('Cron expression (5 fields)').fill('0 9 * * 0');
  await page.getByLabel('Timezone').fill('UTC');
  const preview = page.getByTestId('sched-next-fires');
  await expect(preview).toContainText('Next fires (UTC)', { timeout: 10_000 });
  // `0 9 * * 0` is Sundays — every listed fire says so.
  const items = preview.locator('li');
  await expect(items.first()).toBeVisible();
  for (const text of await items.allTextContents()) expect(text).toMatch(/Sun/);
  // An invalid edit drops the stale preview instead of showing the old times.
  await page.getByLabel('Cron expression (5 fields)').fill('0 9 *');
  await expect(preview).toHaveCount(0);
});
