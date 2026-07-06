import { test, expect } from '@playwright/test';
import { apiCtx, seedWorkspace } from './seed';
import { openPage } from './helpers';

// Postman import in the API client:
//   • the Import button opens the Postman dialog (account sync + file import)
//   • a Postman v2.1 COLLECTION file imports into collections/requests
//   • a Postman ENVIRONMENT export (`{name, values:[…]}`) imports as an API
//     environment (new) — enabled vars kept, disabled dropped
//   • the account-sync endpoint 400s cleanly when no API key is known
//     (the happy path needs a real Postman account, so it isn't e2e-able).
// Desktop-browser project only.

let ws = '';

test.beforeAll(async () => {
  const s = await apiCtx();
  ws = await seedWorkspace(s.ctx, s.base);
  await s.ctx.dispose().catch(() => {});
});

test.beforeEach(async ({ page }, testInfo) => {
  test.skip(testInfo.project.name !== 'desktop-browser', 'desktop-browser only');
  await page.addInitScript((wsId) => {
    if (!localStorage.getItem('otto_workspace')) localStorage.setItem('otto_workspace', wsId as string);
    localStorage.setItem('otto_rail_expanded', '0');
  }, ws);
});

const RUN = Math.random().toString(36).slice(2, 7);

test('imports a Postman collection and environment via the import dialog', async ({ page }) => {
  await openPage(page, 'api');
  await page.getByRole('button', { name: 'Collections' }).first().click();

  // The header Import button opens the Postman dialog with the file fallback.
  // dispatchEvent, not click(): the dialog opens synchronously and its backdrop
  // then covers the button, which makes Playwright's click-retry verification
  // judge the (already landed) click as intercepted and time out.
  await page.getByRole('button', { name: 'Import collections' }).dispatchEvent('click');
  await expect(page.getByText('Sync your whole Postman account')).toBeVisible();
  const fileInput = page.locator('input[type="file"]').first();

  // ── Collection file ───────────────────────────────────────────────────────
  const collection = {
    info: { name: `PM Import ${RUN}`, schema: 'https://schema.getpostman.com/json/collection/v2.1.0/collection.json' },
    item: [
      {
        name: 'Get user',
        request: {
          method: 'GET',
          url: { raw: 'https://api.example.com/users/1' },
          header: [{ key: 'Accept', value: 'application/json' }],
        },
      },
    ],
  };
  await fileInput.setInputFiles({
    name: 'pm.postman_collection.json',
    mimeType: 'application/json',
    buffer: Buffer.from(JSON.stringify(collection)),
  });
  // The dialog closes on file pick; the collection lands in the TREE (not
  // just a toast — scope the assertion to the tree container).
  await expect(page.locator('.backdrop')).toHaveCount(0, { timeout: 15_000 });
  await expect(
    page.locator('.tree').getByText(`PM Import ${RUN}`),
  ).toBeVisible({ timeout: 15_000 });

  // ── Environment file ──────────────────────────────────────────────────────
  await page.getByRole('button', { name: 'Import collections' }).dispatchEvent('click');
  await expect(page.getByText('Sync your whole Postman account')).toBeVisible();
  const environment = {
    name: `PM Env ${RUN}`,
    _postman_variable_scope: 'environment',
    values: [
      { key: 'baseUrl', value: 'https://stg.example.com', enabled: true },
      { key: 'legacy', value: 'off', enabled: false },
    ],
  };
  await page.locator('input[type="file"]').first().setInputFiles({
    name: 'stg.postman_environment.json',
    mimeType: 'application/json',
    buffer: Buffer.from(JSON.stringify(environment)),
  });
  // Success toast names the env with the ENABLED variable count only (1).
  await expect(page.getByText('Environment imported')).toBeVisible({ timeout: 15_000 });
  await expect(page.getByText(`PM Env ${RUN} · 1 variable(s)`)).toBeVisible();
});

test('account sync without a key → 400 with a pointer to Postman settings', async () => {
  const { ctx, base } = await apiCtx();
  const r = await ctx.post(`${base}/api/v1/workspaces/${ws}/api-client/postman/sync`, {
    data: {},
  });
  expect(r.status()).toBe(400);
  const body = await r.json();
  expect(String(body.message)).toContain('Postman API key');
  await ctx.dispose().catch(() => {});
});
