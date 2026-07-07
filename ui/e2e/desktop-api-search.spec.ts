import { test, expect, type APIRequestContext } from '@playwright/test';
import { apiCtx, seedWorkspace } from './seed';
import { openPage } from './helpers';

// ─────────────────────────────────────────────────────────────────────────────
// API client — sidebar search (2026-07-07):
//   • Collections tab: a search box filters saved requests by name / URL /
//     method and collections by name. Ancestor folders stay visible, branches
//     force-open while filtering, and clearing restores the full tree.
//   • History tab: a search box filters entries by URL (domain / path / query
//     string), method, and status code.
//
// Desktop-browser project only.
// ─────────────────────────────────────────────────────────────────────────────

let ws = '';

async function postJson(ctx: APIRequestContext, url: string, data: unknown): Promise<any> {
  const r = await ctx.post(url, { data });
  if (!r.ok()) throw new Error(`POST ${url} → ${r.status()} ${await r.text()}`);
  return r.json();
}

test.beforeAll(async () => {
  const s = await apiCtx();
  ws = await seedWorkspace(s.ctx, s.base);
  const api = `${s.base}/api/v1/workspaces/${ws}/api-client`;

  // Two-level tree + an ungrouped request.
  const payments = await postJson(s.ctx, `${api}/collections`, { name: 'Payments' });
  const refunds = await postJson(s.ctx, `${api}/collections`, {
    name: 'Refunds',
    parent_id: payments.id,
  });
  await postJson(s.ctx, `${api}/requests`, {
    collection_id: payments.id,
    name: 'List users',
    method: 'GET',
    url: 'https://api.staging.example.com/v1/users?limit=10',
  });
  await postJson(s.ctx, `${api}/requests`, {
    collection_id: refunds.id,
    name: 'Create refund',
    method: 'POST',
    url: 'https://api.staging.example.com/v1/refunds',
  });
  await postJson(s.ctx, `${api}/requests`, {
    collection_id: null,
    name: 'Health check',
    method: 'GET',
    url: 'https://ops.internal.example.net/healthz',
  });

  // History rows: a failed execute is still recorded (null status), and the
  // `.invalid` TLD fails DNS immediately — fast + fully offline.
  for (const [method, url] of [
    ['GET', 'http://alpha.e2e-nowhere.invalid/users?id=42'],
    ['POST', 'http://beta.e2e-nowhere.invalid/orders/checkout'],
  ] as const) {
    await s.ctx.post(`${api}/execute`, { data: { method, url, timeout_ms: 2000 } });
  }
  await s.ctx.dispose().catch(() => {});
});

test.beforeEach(async ({ page }, testInfo) => {
  test.skip(testInfo.project.name !== 'desktop-browser', 'desktop-browser only');
  await page.addInitScript((wsId) => {
    if (!localStorage.getItem('otto_workspace')) localStorage.setItem('otto_workspace', wsId as string);
    localStorage.setItem('otto_rail_expanded', '0');
  }, ws);
});

test('collections search filters by name / URL / method and restores on clear', async ({ page }) => {
  await openPage(page, 'api');
  const input = page.getByLabel('Search collections and requests');
  const rows = page.locator('.req-row');
  await expect(rows).toHaveCount(3);

  // Name match keeps the ancestor chain (Payments ▸ Refunds) visible.
  await input.fill('refund');
  await expect(rows).toHaveCount(1);
  await expect(page.getByText('Create refund')).toBeVisible();
  await expect(page.getByText('Payments')).toBeVisible();
  await expect(page.getByText('Refunds')).toBeVisible();
  await expect(page.getByText('List users')).toBeHidden();

  // URL/domain match reaches an ungrouped request.
  await input.fill('ops.internal');
  await expect(rows).toHaveCount(1);
  await expect(page.getByText('Health check')).toBeVisible();

  // Multi-token: method + URL fragment must BOTH match.
  await input.fill('post staging');
  await expect(rows).toHaveCount(1);
  await expect(page.getByText('Create refund')).toBeVisible();

  // Collection-name match keeps the whole branch.
  await input.fill('payments');
  await expect(rows).toHaveCount(2);

  await input.fill('no-such-request-zzqx');
  await expect(page.locator('.no-match')).toBeVisible();

  await input.fill('');
  await expect(rows).toHaveCount(3);
});

test('history search filters by URL / method / path+query', async ({ page }) => {
  await openPage(page, 'api');
  await page.getByRole('tab', { name: 'History' }).click();
  const input = page.getByLabel('Search request history');
  const rows = page.locator('.hist-row');
  await expect(rows).toHaveCount(2);

  await input.fill('alpha'); // domain
  await expect(rows).toHaveCount(1);
  await expect(page.getByText('alpha.e2e-nowhere.invalid')).toBeVisible();

  await input.fill('checkout'); // path
  await expect(rows).toHaveCount(1);
  await expect(page.getByText('beta.e2e-nowhere.invalid')).toBeVisible();

  await input.fill('id=42'); // query string
  await expect(rows).toHaveCount(1);

  await input.fill('post'); // method (also matches nothing else here)
  await expect(rows).toHaveCount(1);

  await input.fill('no-such-entry-zzqx');
  await expect(page.getByText('No history matches')).toBeVisible();

  await input.fill('');
  await expect(rows).toHaveCount(2);
});
