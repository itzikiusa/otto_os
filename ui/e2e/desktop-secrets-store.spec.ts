import { test, expect, type Page } from '@playwright/test';
import { apiCtx } from './seed';

// ─────────────────────────────────────────────────────────────────────────────
// Settings ▸ Trust & safety ▸ Secret storage (root only).
//   • the isolated test daemon runs OTTO_SECRETS=file → the real status says
//     "plaintext store active" with no migration offered while it holds no
//     secrets, and the warning + "Secure secrets…" once another spec (the
//     daemon is shared) has stored some;
//   • with plaintext secrets present (mocked status) a warning banner and the
//     explicit "Secure secrets…" action appear; it confirms first, POSTs
//     {confirm: true} and the card then shows the encrypted store;
//   • a locked Keychain (mocked 502) shows an inline error that says nothing
//     changed.
// The migration itself is unit-tested in otto-keychain (verify-before-delete,
// tamper detection, no plaintext left); mocking keeps this spec away from the
// real macOS Keychain.
// ─────────────────────────────────────────────────────────────────────────────

test.use({ serviceWorkers: 'block' });

test.beforeEach(async ({}, info) => {
  test.skip(info.project.name !== 'desktop-browser', 'desktop Settings');
});

const card = (page: Page) => page.getByRole('region', { name: 'Secret storage' });

async function boot(page: Page): Promise<void> {
  await page.goto('/#/settings/trust-safety');
  await expect(page.locator('.shell')).toBeVisible({ timeout: 60_000 });
}

const plaintext = {
  mode: 'plaintext',
  plaintext_file: true,
  plaintext_entries: 3,
  key_state: 'not_loaded',
  migration_available: true,
  migrating: false,
  backup_present: false,
};
const encrypted = {
  ...plaintext,
  mode: 'encrypted',
  plaintext_file: false,
  plaintext_entries: 0,
  key_state: 'unlocked',
  migration_available: false,
};

test('real daemon: plaintext store active, nothing to migrate yet', async ({ page }) => {
  // The daemon is shared by every spec, so whether it holds secrets yet
  // depends on run order: read its real status and hold the card to it.
  const { ctx, base } = await apiCtx();
  const r = await ctx.get(`${base}/api/v1/admin/secrets/status`);
  expect(r.ok(), await r.text()).toBeTruthy();
  const status = (await r.json()) as { mode: string; plaintext_entries: number };
  await ctx.dispose();
  expect(status.mode).toBe('plaintext');
  await boot(page);
  await expect(card(page)).toBeVisible();
  await expect(card(page).getByText('Plaintext file (not encrypted)')).toBeVisible();
  if (status.plaintext_entries === 0) {
    await expect(card(page).getByText('The plaintext secret store is active.')).toBeVisible();
    await expect(card(page).getByRole('button', { name: /Secure secrets/ })).toHaveCount(0);
  } else {
    const n = status.plaintext_entries;
    await expect(card(page).getByRole('alert')).toContainText(
      `${n} secret${n === 1 ? ' is' : 's are'} in a plaintext file`,
    );
    await expect(card(page).getByRole('button', { name: /Secure secrets/ })).toBeVisible();
  }
});

test('plaintext secrets: banner, confirm, migrate, encrypted', async ({ page }) => {
  let migrated = false;
  let body: unknown = null;
  await page.route('**/api/v1/admin/secrets/status', (r) =>
    r.fulfill({ json: migrated ? encrypted : plaintext }),
  );
  await page.route('**/api/v1/admin/secrets/secure', async (r) => {
    body = r.request().postDataJSON();
    migrated = true;
    await r.fulfill({ json: { migrated: 3, total: 3, duration_ms: 12 } });
  });
  await boot(page);
  await expect(card(page).getByRole('alert')).toContainText('3 secrets are in a plaintext file');
  await card(page).getByRole('button', { name: 'Secure secrets…' }).click();
  // Confirms first: cancelling sends nothing.
  await page.getByRole('button', { name: 'Cancel' }).click();
  expect(body).toBeNull();
  await card(page).getByRole('button', { name: 'Secure secrets…' }).click();
  await page.getByRole('button', { name: 'Encrypt secrets' }).click();
  await expect.poll(() => body).toEqual({ confirm: true });
  await expect(card(page).getByText('Encrypted file, key in the macOS Keychain')).toBeVisible();
  await expect(card(page).getByRole('alert')).toHaveCount(0);
  await expect(card(page).getByText('Unlocked')).toBeVisible();
});

test('locked Keychain: inline error, nothing changed', async ({ page }) => {
  await page.route('**/api/v1/admin/secrets/status', (r) => r.fulfill({ json: plaintext }));
  await page.route('**/api/v1/admin/secrets/secure', (r) =>
    r.fulfill({ status: 502, json: { error: 'secret store locked: the macOS Keychain did not answer' } }),
  );
  await boot(page);
  await card(page).getByRole('button', { name: 'Secure secrets…' }).click();
  await page.getByRole('button', { name: 'Encrypt secrets' }).click();
  await expect(card(page).getByText(/Secrets were not changed/)).toBeVisible();
  await expect(card(page).getByRole('button', { name: 'Secure secrets…' })).toBeEnabled();
});
