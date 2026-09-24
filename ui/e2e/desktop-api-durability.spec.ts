import { test, expect, type Page } from '@playwright/test';
import { apiCtx, seedWorkspace } from './seed';
import { openApiEditor, openPage } from './helpers';

// ─────────────────────────────────────────────────────────────────────────────
// API client — durability & secrets (2026-07-04 design):
//   • Scripts / Docs / Settings persist WITH the saved request (`extras`) —
//     loading it fresh after a full reload restores them (server-side, not the
//     local tab cache: we load through the collections tree, and also assert
//     the stored row itself via the REST API).
//   • A saved bearer token is Keychain-migrated: the row returns a `$secret`
//     marker (never the plaintext), and the builder renders it masked.
//   • Environment secret lock: a locked variable's value is write-only — GET
//     returns the key in `secret_keys` and no value in `variables`.
//
// Desktop-browser project only.
// ─────────────────────────────────────────────────────────────────────────────

let ws = '';
let base = '';
let token = '';

test.beforeAll(async () => {
  const s = await apiCtx();
  base = s.base;
  token = s.token;
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

const urlInput = (page: Page) => page.getByLabel('Request URL');
const builderTab = (page: Page, name: string) =>
  page.locator('.builder').getByRole('tab', { name, exact: true });

/** Type into a CodeMirror editor inside `scope`. */
async function typeInEditor(scope: ReturnType<Page['locator']>, text: string): Promise<void> {
  const cm = scope.locator('.cm-content').first();
  await cm.click();
  await cm.pressSequentially(text);
}

test('scripts/docs/settings persist with the saved request (server-side)', async ({ page }) => {
  await openApiEditor(page);
  const name = `durability-${Date.now()}`;

  await urlInput(page).fill('https://example.com/durable');

  // Scripts tab: a pre-request script.
  await builderTab(page, 'Scripts').click();
  await typeInEditor(page.locator('.script-block').first(), "pm.environment.set('who','otto')");

  // Docs tab: markdown notes.
  await builderTab(page, 'Docs').click();
  await typeInEditor(page.locator('.docs-edit'), '# Durable docs');

  // Settings tab: non-default timeout + redirects off.
  await builderTab(page, 'Settings').click();
  await page.locator('.set-num').fill('12345');
  await page.locator('.set-row.toggle').first().locator('input[type=checkbox]').uncheck();

  // Save (name prompt; no collections exist → no collection prompt).
  await page.getByRole('button', { name: 'Save', exact: true }).click();
  const dlg = page.getByRole('dialog');
  await dlg.getByRole('textbox').fill(name);
  await dlg.getByRole('button', { name: 'Save', exact: true }).click();
  await expect(page.getByText('Request saved')).toBeVisible();

  // The stored ROW carries the extras (this is the durability guarantee —
  // assert via REST, independent of any local tab cache).
  const rows = await page.evaluate(
    async ({ base, token, ws }) => {
      const r = await fetch(`${base}/api/v1/workspaces/${ws}/api-client/requests`, {
        headers: { Authorization: `Bearer ${token}` },
      });
      return r.json();
    },
    { base, token, ws },
  );
  const saved = rows.find((r: { name: string }) => r.name === name);
  expect(saved).toBeTruthy();
  expect(saved.extras?.scripts?.pre).toContain("pm.environment.set('who','otto')");
  expect(saved.extras?.docs_md).toContain('Durable docs');
  expect(saved.extras?.settings?.timeout_ms).toBe(12345);
  expect(saved.extras?.settings?.follow_redirects).toBe(false);

  // Full reload, then load the request fresh from the sidebar tree: the
  // once-draft-only fields come back.
  await page.reload();
  await openPage(page, 'api');
  await page.locator('.req-tab-new').click(); // pristine tab (not the cached one)
  await page.getByText(name, { exact: true }).first().click();

  await expect(urlInput(page)).toHaveValue('https://example.com/durable');
  await builderTab(page, 'Settings').click();
  await expect(page.locator('.set-num')).toHaveValue('12345');
  await expect(
    page.locator('.set-row.toggle').first().locator('input[type=checkbox]'),
  ).not.toBeChecked();
  await builderTab(page, 'Docs').click();
  await expect(page.locator('.docs-edit .cm-content')).toContainText('Durable docs');
  await builderTab(page, 'Scripts').click();
  await expect(page.locator('.script-block').first().locator('.cm-content')).toContainText(
    'pm.environment.set',
  );
});

test('saved bearer token is Keychain-migrated and rendered masked', async ({ page }) => {
  await openPage(page, 'api');
  const name = `secret-${Date.now()}`;

  await urlInput(page).fill('https://example.com/secret');
  await builderTab(page, 'Auth').click();
  await page.getByLabel('Auth type').selectOption('bearer');
  await page.locator('#auth-token').fill('super-secret-token');

  await page.getByRole('button', { name: 'Save', exact: true }).click();
  const dlg = page.getByRole('dialog');
  await dlg.getByRole('textbox').fill(name);
  await dlg.getByRole('button', { name: 'Save', exact: true }).click();
  await expect(page.getByText('Request saved')).toBeVisible();

  // The row holds a marker, not the token.
  const rows = await page.evaluate(
    async ({ base, token, ws }) => {
      const r = await fetch(`${base}/api/v1/workspaces/${ws}/api-client/requests`, {
        headers: { Authorization: `Bearer ${token}` },
      });
      return r.json();
    },
    { base, token, ws },
  );
  const saved = rows.find((r: { name: string }) => r.name === name);
  expect(saved.auth.token).toEqual({ $secret: `otto.api.request.${saved.id}` });
  expect(JSON.stringify(saved)).not.toContain('super-secret-token');

  // The builder now shows the masked placeholder (draft re-adopted the marker).
  await expect(page.locator('#auth-token')).toHaveValue('');
  await expect(page.locator('#auth-token')).toHaveAttribute('placeholder', /stored in Keychain/);
});

test('environment secret lock: value is write-only', async ({ page }) => {
  await openApiEditor(page);

  // Create an environment from the header's environment switcher.
  await page.getByRole('button', { name: /^Environment:/ }).click();
  await page.getByRole('menuitem', { name: 'New environment…' }).click();
  const dlg = page.getByRole('dialog');
  await dlg.getByRole('textbox').fill('e2e-secrets');
  await dlg.getByRole('button', { name: 'Create', exact: true }).click();
  await expect(page.getByRole('heading', { name: 'e2e-secrets' })).toBeVisible();

  // Variables table: typing into the trailing blank row adds a new one.
  const rows = page.locator('.var-row');
  await rows.last().locator('.var-key').fill('base');
  await rows.nth(-2).locator('.var-val').fill('https://x');
  await rows.last().locator('.var-key').fill('api_token');
  await rows.nth(-2).locator('.var-val').fill('hush-hush');
  await rows.nth(-2).getByRole('button', { name: 'Toggle secret' }).click();
  await page.getByRole('button', { name: 'Save changes' }).click();

  // GET returns the key name only — never the value.
  await expect.poll(async () =>
    page.evaluate(
      async ({ base, token, ws }) => {
        const r = await fetch(`${base}/api/v1/workspaces/${ws}/api-client/environments`, {
          headers: { Authorization: `Bearer ${token}` },
        });
        const envs = await r.json();
        const env = envs.find((e: { name: string }) => e.name === 'e2e-secrets');
        return env
          ? { keys: env.secret_keys, hasValue: JSON.stringify(env).includes('hush-hush'), base: env.variables.base }
          : null;
      },
      { base, token, ws },
    ),
  ).toEqual({ keys: ['api_token'], hasValue: false, base: 'https://x' });

  // After saving, the secret renders masked (empty value + lock on); secrets
  // are listed after plain variables, before the trailing blank row.
  const secretRow = rows.nth(-2);
  await expect(secretRow.locator('.var-key')).toHaveValue('api_token');
  await expect(secretRow.locator('.var-val')).toHaveValue('');
  await expect(secretRow.locator('.var-val')).toHaveAttribute('placeholder', /stored in Keychain/);
});
