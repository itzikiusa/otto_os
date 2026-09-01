import { test, expect, type APIRequestContext, type Page } from '@playwright/test';
import { mkdtempSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { apiCtx, seedWorkspace } from './seed';

// Browser module: DOM marks, the notes rail, send-to-session, and vault save.
//
// The daemon's `/workspaces/{wid}/browser/page` route is netguard-checked and
// really fetches the given URL server-side (crates/otto-server/src/routes/
// browser.rs) — loopback/private targets are REJECTED by design (SSRF
// defense, see crates/otto-netguard), so this spec never points it at a
// locally-served fixture. Instead it intercepts the front-end's GET to that
// route with `page.route(...)` and fulfills a canned `BrowserPage` JSON body
// directly in the browser context — the daemon's real fetch path (and its
// netguard check) is never exercised here; that's covered by the Rust tests
// in routes/browser.rs. Tab/annotation creation, send-to-session, and
// vault-save are all REAL calls against the isolated test daemon — none of
// them fetch a URL server-side (annotations just store a selector; send
// writes into a session's input; vault-save is steered onto its
// caller-supplied-summary branch, which skips the fetch — see BrowserView's
// `doVaultSave`).

const FIXTURE_URL = 'https://example.invalid/fixture-page';

test.describe.configure({ mode: 'serial' });

let ctx: APIRequestContext;
let base: string;
let wsId = '';

test.beforeEach(async ({ page }, info) => {
  test.skip(info.project.name !== 'desktop-browser', 'desktop-browser project only');
  const c = await apiCtx();
  ctx = c.ctx;
  base = c.base;
  wsId = await seedWorkspace(ctx, base);

  // Mock the reader-mode page fetch: the frontend's GET .../browser/page?url=…
  // is answered here, in the browser context, before it ever reaches the
  // daemon — see the file header for why.
  await page.route('**/browser/page?url=**', (route) =>
    route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({
        url: FIXTURE_URL,
        title: 'Fixture Page',
        markdown: 'Some fixture body content for the E2E reader test.',
        html: '<p>Some fixture body content for the E2E reader test.</p>',
        engine: 'mock',
        degraded: false,
      }),
    }),
  );

  await page.addInitScript((id) => localStorage.setItem('otto_workspace', id as string), wsId);
  await page.goto('/#/browser');
  await expect(page.locator('.shell')).toBeVisible({ timeout: 15_000 });
});

test.afterEach(async () => {
  await ctx?.dispose();
});

/** Navigate the URL bar to the mocked fixture URL and wait for the reader to
 *  render it. */
async function openFixture(page: Page): Promise<void> {
  await page.getByPlaceholder('Enter URL').fill(FIXTURE_URL);
  await page.getByTitle('Go').click();
  await expect(page.locator('.reader h1')).toHaveText('Fixture Page', { timeout: 15_000 });
}

test('mark → note → rail shows it', async ({ page }) => {
  await openFixture(page);

  await page.getByRole('button', { name: 'Mark element' }).click();
  await page.locator('.reader h1').click();
  await page.getByPlaceholder('Add a note').fill('interesting');
  await page.getByRole('button', { name: 'Save mark' }).click();

  await expect(page.locator('.notes-rail')).toContainText('interesting');
  // The marked element itself gets a highlight class once the annotation
  // round-trips and ReaderView re-resolves its selector against the DOM.
  await expect(page.locator('.reader h1')).toHaveClass(/marked/);
});

test('send-to-session posts the annotation id + chosen session', async ({ page }) => {
  const sessionResp = await ctx.post(`${base}/api/v1/workspaces/${wsId}/sessions`, {
    data: { kind: 'agent', provider: 'shell', title: 'Browser E2E target', cwd: '/tmp' },
  });
  expect(sessionResp.ok()).toBeTruthy();
  const sessionId = ((await sessionResp.json()) as { id: string }).id;

  await openFixture(page);
  await page.getByRole('button', { name: 'Mark element' }).click();
  await page.locator('.reader h1').click();
  await page.getByPlaceholder('Add a note').fill('send me');
  await page.getByRole('button', { name: 'Save mark' }).click();
  await expect(page.locator('.notes-rail')).toContainText('send me');

  const row = page.locator('.notes-rail .row', { hasText: 'send me' });
  const sendReq = page.waitForRequest(
    (req) => req.url().includes('/annotations/') && req.url().includes('/send') && req.method() === 'POST',
  );
  await row.getByTitle('Send to session').click();
  await page.getByRole('menuitem', { name: /Browser E2E target/ }).click();
  const req = await sendReq;
  expect(req.postDataJSON()).toEqual({ session_id: sessionId });
});

test('save to vault writes a note without re-fetching the page', async ({ page }) => {
  const dir = mkdtempSync(join(tmpdir(), 'otto-e2e-browser-vault-'));
  const vaultResp = await ctx.post(`${base}/api/v1/workspaces/${wsId}/vault/vaults`, {
    data: { name: 'Browser E2E Vault', root_path: dir },
  });
  expect(vaultResp.ok(), `create vault → ${vaultResp.status()} ${await vaultResp.text()}`).toBeTruthy();

  await openFixture(page);
  const vaultSaveReq = page.waitForRequest(
    (req) => req.url().includes('/vault-save') && req.method() === 'POST',
  );
  await page.getByTitle('Save to vault').click();
  const req = await vaultSaveReq;
  const body = req.postDataJSON() as { url: string; vault_id: number; summary?: string };
  expect(body.url).toBe(FIXTURE_URL);
  expect(body.summary, 'vault-save must carry a client-supplied summary to skip the server fetch').toBeTruthy();

  await expect(page.locator('.toasts')).toContainText('Saved to vault', { timeout: 10_000 });
});
