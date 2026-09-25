import { test, expect } from '@playwright/test';
import { apiCtx, seedWorkspace } from './seed';

test.use({ serviceWorkers: 'block' });
test.beforeEach(async ({}, info) => {
  test.skip(info.project.name !== 'desktop-browser', 'desktop audit with explicit responsive viewports');
});

test('MCP arrow keys move focus together with the selected tab', async ({ page }) => {
  await page.goto('/#/mcp');
  const otto = page.getByRole('tab', { name: 'Otto server' });
  const external = page.getByRole('tab', { name: /External servers/ });
  await otto.focus();
  await page.keyboard.press('ArrowRight');
  await expect(external).toHaveAttribute('aria-selected', 'true');
  await expect(external).toBeFocused();
  await page.keyboard.press('End');
  await expect(page.getByRole('tab', { name: /Activity/ })).toBeFocused();
  await page.keyboard.press('Home');
  await expect(otto).toBeFocused();
});

test('By-user membership load failure blocks role edits and offers Retry', async ({ page }) => {
  const { ctx, base } = await apiCtx();
  await seedWorkspace(ctx, base);
  const user = await ctx.post(`${base}/api/v1/users`, { data: {
    username: `audit-${Date.now()}`, display_name: 'Audit user', password: 'audit-password-123',
  }});
  expect(user.ok()).toBeTruthy();
  let fail = true;
  await page.route('**/api/v1/workspaces/*/members', async route => {
    if (fail && route.request().method() === 'GET') {
      await route.fulfill({ status: 503, contentType: 'application/json', body: JSON.stringify({ error: 'Membership temporarily unavailable' }) });
    } else await route.continue();
  });
  await page.goto('/#/settings/users');
  await page.getByRole('button', { name: 'By user', exact: true }).click();
  const error = page.getByTestId('load-error');
  await expect(error).toContainText('workspace memberships');
  await expect(page.getByRole('group', { name: 'Set the role in every workspace' })).toHaveCount(0);
  fail = false;
  await error.getByRole('button', { name: 'Retry', exact: true }).click();
  await expect(page.getByRole('group', { name: 'Set the role in every workspace' })).toBeVisible();
  await ctx.dispose();
});

test('holding Enter creates just one personal access token', async ({ page }) => {
  let posts = 0;
  let release!: () => void;
  const held = new Promise<void>(resolve => { release = resolve; });
  await page.route('**/api/v1/auth/tokens', async route => {
    if (route.request().method() !== 'POST') return route.continue();
    posts++;
    await held;
    await route.fulfill({ status: 503, contentType: 'application/json', body: JSON.stringify({ error: 'Audit unavailable' }) });
  });
  await page.goto('/#/settings/tokens');
  const label = page.getByLabel('Label (optional)');
  await label.fill('Audit token');
  await label.press('Enter');
  await expect(page.getByRole('button', { name: 'Creating…' })).toBeDisabled();
  await label.press('Enter');
  await label.press('Enter');
  try { expect(posts).toBe(1); } finally { release(); }
});

test('Plugins load failure recovers with Retry on a phone', async ({ page }) => {
  await page.setViewportSize({ width: 375, height: 812 });
  let fail = true;
  await page.route('**/api/v1/plugin-admin', route => fail
    ? route.fulfill({ status: 503, contentType: 'application/json', body: JSON.stringify({ error: 'Temporarily unavailable' }) })
    : route.fulfill({ json: [] }));
  await page.goto('/#/settings/plugins');
  const error = page.getByTestId('load-error');
  await expect(error).toBeVisible();
  fail = false;
  await error.getByRole('button', { name: 'Retry' }).click();
  await expect(page.getByText('No plugins installed', { exact: true })).toBeVisible();
  await expect(page.getByLabel('Plugin source')).toBeVisible();
  await page.screenshot({ path: '/tmp/otto-ux-settings-plugins-phone.png' });
});

test('Settings sections remain reachable and titled at desktop and phone widths', async ({ page }) => {
  test.setTimeout(120_000);
  await page.goto('/#/settings/appearance');
  const nav = page.locator('.settings-nav');
  await expect(nav.locator('.settings-nav-item').first()).toBeVisible();
  const labels = await nav.locator('.settings-nav-label').allTextContents();
  for (const width of [1440, 375]) {
    await page.setViewportSize({ width, height: 900 });
    for (const label of labels) {
      await page.getByRole('searchbox', { name: 'Filter settings' }).fill(label);
      await nav.getByRole('button', { name: new RegExp(`^${label.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')}`) }).first().click();
      await expect(page.locator('.settings-body h1.ph-title')).toHaveText(label);
    }
  }
});

test('Skills Lab search opens a library skill and its Files tab', async ({ page }) => {
  const { ctx, base } = await apiCtx();
  const name = `ux-audit-${Date.now()}`;
  const created = await ctx.post(`${base}/api/v1/library/skills`, { data: {
    name, category: 'review', description: 'Audit skill for browser navigation',
    body: '---\ndescription: Audit skill for browser navigation\n---\n\n## Audit workflow\n\nReview the supplied content.',
  }});
  expect(created.ok()).toBeTruthy();
  await page.goto('/#/skills-eval');
  await page.getByRole('searchbox', { name: 'Search skills' }).fill(name);
  const row = page.getByTestId('skill-row');
  await expect(row).toHaveCount(1);
  await row.click();
  await expect(page.getByTestId('skill-name')).toHaveText(name);
  await page.getByRole('tab', { name: 'Files', exact: true }).click();
  await expect(page.getByTestId('skill-editor')).toContainText('Audit workflow');
  await expect(page.getByTestId('save-skill')).toBeDisabled();
  await ctx.dispose();
});
