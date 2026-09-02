import { test, expect, type Page } from '@playwright/test';
import { apiCtx } from './seed';
import { expectFullyInViewport, openPage } from './helpers';

// ─────────────────────────────────────────────────────────────────────────────
// AWS console (desktop-browser only).
//
// 1. `#/aws` renders either the first-run install panel (CLI missing) or the
//    accounts overview — with zero console errors.
// 2. The "Add account" wizard is a Modal: it must be FULLY inside the viewport
//    (the floating-UI rule in AGENTS.md) — checked with expectFullyInViewport.
// 3. Seeding an `access_keys` account through the API renders a card with its
//    name + environment pill.
//
// The backend lands in parallel with this UI: when the isolated daemon has no
// `/aws/status` route (404) the API-dependent tests skip with a logged reason
// instead of failing.
// ─────────────────────────────────────────────────────────────────────────────

const ACCOUNT_NAME = `e2e-aws-${Date.now().toString(36)}`;

interface Backend {
  present: boolean;
  installed: boolean;
  reason: string;
}

let backend: Backend = { present: false, installed: false, reason: 'not probed' };

test.beforeAll(async () => {
  const { ctx, base } = await apiCtx();
  try {
    const r = await ctx.get(`${base}/api/v1/aws/status`);
    if (r.status() === 404) {
      backend = { present: false, installed: false, reason: 'GET /api/v1/aws/status → 404 (AWS routes not in this daemon yet)' };
    } else if (!r.ok()) {
      backend = { present: false, installed: false, reason: `GET /api/v1/aws/status → ${r.status()}` };
    } else {
      const body = (await r.json()) as { installed?: boolean };
      backend = { present: true, installed: body.installed === true, reason: '' };
    }
  } catch (e) {
    backend = { present: false, installed: false, reason: `status probe failed: ${String(e)}` };
  } finally {
    await ctx.dispose();
  }
  if (!backend.present) console.log(`[desktop-aws] skipping API-dependent tests: ${backend.reason}`);
});

function collectErrors(page: Page): string[] {
  const errors: string[] = [];
  page.on('console', (m) => {
    if (m.type() === 'error') errors.push(m.text());
  });
  page.on('pageerror', (e) => errors.push(`pageerror: ${e.message}`));
  return errors;
}

test('#/aws renders the install panel or the accounts overview without console errors', async ({ page }) => {
  const errors = collectErrors(page);
  await openPage(page, 'aws');
  const install = page.getByTestId('aws-install-panel');
  const overview = page.getByRole('heading', { name: 'AWS', level: 1 });
  const unavailable = page.getByText('AWS console unavailable');
  await expect(install.or(overview).or(unavailable).first()).toBeVisible({ timeout: 15_000 });
  if (!backend.present) {
    // Daemon without the routes: the page must degrade to its explicit
    // "unavailable" empty state, never a blank pane or a thrown error.
    await expect(unavailable).toBeVisible();
  }
  // Network 404s from a not-yet-built backend are expected; anything else is a bug.
  const real = errors.filter((e) => !/404|Failed to load resource|aws\/status/i.test(e));
  expect(real, `console errors: ${real.join('\n')}`).toEqual([]);
});

test('the Add-account wizard opens fully inside the viewport', async ({ page }) => {
  test.skip(!backend.present, backend.reason);
  test.skip(!backend.installed, 'aws CLI not installed on the test daemon — the overview (and its wizard) is behind the install panel');
  await openPage(page, 'aws');
  await page.getByTestId('aws-add-account').click();
  const dialog = page.getByRole('dialog', { name: 'Add AWS account' });
  await expect(dialog).toBeVisible();
  await expect(page.getByTestId('aws-account-wizard')).toBeVisible();
  await page.waitForTimeout(150); // sheet-in animation
  await expectFullyInViewport(page, dialog, 'add-account wizard');
  // Step 1 offers both credential sources.
  await expect(dialog.getByRole('tab', { name: 'Use an existing AWS profile' })).toBeVisible();
  await expect(dialog.getByRole('tab', { name: 'Enter access keys' })).toBeVisible();
  await page.keyboard.press('Escape');
  await expect(dialog).toBeHidden();
});

test('an access_keys account seeded via the API renders as a card with its environment pill', async ({ page }) => {
  test.skip(!backend.present, backend.reason);
  test.skip(!backend.installed, 'aws CLI not installed on the test daemon — cards are behind the install panel');
  const { ctx, base } = await apiCtx();
  let id = '';
  try {
    const r = await ctx.post(`${base}/api/v1/aws/accounts`, {
      data: {
        name: ACCOUNT_NAME,
        auth_mode: 'access_keys',
        region: 'eu-west-1',
        access_key_id: 'AKIAE2EFAKEKEY000000',
        secret_access_key: 'e2e-fake-secret-not-real-0000000000000000',
        environment: 'prod',
        color: '#ef4444',
      },
    });
    expect(r.ok(), `POST /aws/accounts → ${r.status()} ${await r.text()}`).toBeTruthy();
    id = ((await r.json()) as { id: string }).id;

    await openPage(page, 'aws');
    const card = page.getByTestId('aws-account-card').filter({ hasText: ACCOUNT_NAME });
    await expect(card).toBeVisible({ timeout: 15_000 });
    const pill = card.locator('.env-pill');
    await expect(pill).toHaveText(/prod/i);
    await expect(pill).toHaveAttribute('data-env', 'prod');
    // Deep link into a service view for the new account renders its toolbar.
    await page.goto(`/#/aws/${id}/s3`);
    await expect(page.getByRole('heading', { name: 'S3', level: 2 })).toBeVisible({ timeout: 15_000 });
  } finally {
    if (id) await ctx.delete(`${base}/api/v1/aws/accounts/${id}`).catch(() => {});
    await ctx.dispose();
  }
});
