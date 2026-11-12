import { test, expect, type Page } from '@playwright/test';
import { apiCtx, seedWorkspace } from './seed';
import { expectNoHorizontalOverflow } from './helpers';

// One home for MCP servers: the Control Plane's Servers list shows native
// servers (launched by the agent CLI from `.mcp.json`) beside governed ones,
// each with its "Runs" mode; a native one switches to governed (and back)
// behind a confirm that names the tool-name change; Settings ▸ MCP servers
// redirects here. Runs against the real daemon (the switch is a real PATCH).

let base = '';
let workspace = '';
let nativeId = '';

test.describe.configure({ mode: 'serial' });

test.beforeAll(async () => {
  const seeded = await apiCtx();
  base = seeded.base;
  workspace = await seedWorkspace(seeded.ctx, base, 'MCP one home');
  // What Settings ▸ MCP servers' "Add server" made: a native server.
  const res = await seeded.ctx.post(`${base}/api/v1/workspaces/${workspace}/mcp-servers`, {
    data: { name: 'native-fixture', command: 'true', args: ['--fixture'], env: { LOG_LEVEL: 'info' }, enabled: true },
  });
  expect(res.ok(), `create native server → ${res.status()} ${await res.text()}`).toBeTruthy();
  nativeId = (await res.json()).id as string;
  await seeded.ctx.dispose();
});

test.beforeEach(async ({ page }) => {
  await page.addInitScript((wsId) => {
    if (!sessionStorage.getItem('mcp-one-home-seeded')) {
      localStorage.setItem('otto_workspace', wsId as string);
      sessionStorage.setItem('mcp-one-home-seeded', '1');
    }
  }, workspace);
});

function row(page: Page, name: string) {
  return page.getByTestId('mcp-server-row').filter({ hasText: name });
}

test('Settings ▸ MCP servers lands on the Control Plane’s Servers page', async ({ page }) => {
  await page.goto('/#/settings/mcp-servers');
  await expect.poll(() => page.evaluate(() => location.hash), { timeout: 30_000 }).toBe('#/mcp/servers');
  await expect(row(page, 'native-fixture')).toBeVisible({ timeout: 30_000 });
  // Out of the Settings list.
  await page.goto('/#/settings/appearance');
  const labels = page.locator('.settings-nav-item .settings-nav-label');
  await expect(labels.filter({ hasText: 'Appearance' }).first()).toBeVisible({ timeout: 30_000 });
  await expect(labels.filter({ hasText: /^MCP servers$/ })).toHaveCount(0);
});

test('a native server shows its mode and the Not governed badge', async ({ page }) => {
  await page.goto('/#/mcp/servers');
  const r = row(page, 'native-fixture');
  await expect(r).toHaveAttribute('data-runs', 'native', { timeout: 30_000 });
  await expect(r.getByTestId('mcp-server-runs-badge')).toContainText('Runs: Native');
  await expect(r.getByText('Not governed', { exact: true })).toBeVisible();
  await expect(r).toContainText('no policy, approval or audit applies');
  // Native rows edit (no Discover: Otto never connects to them).
  await expect(r.getByRole('button', { name: 'Discover', exact: true })).toHaveCount(0);
  await r.getByRole('button', { name: 'Edit', exact: true }).click();
  const dialog = page.getByRole('dialog');
  await expect(dialog).toContainText('Edit native MCP server');
  await expect(dialog.getByRole('textbox').first()).toHaveValue('native-fixture');
  await dialog.getByRole('button', { name: 'Cancel', exact: true }).click();
  await expect(dialog).toHaveCount(0);
});

test('switching to governed confirms the tool-name change, and switching back reverses it', async ({ page }) => {
  await page.goto('/#/mcp/servers');
  const r = row(page, 'native-fixture');
  await expect(r).toHaveAttribute('data-runs', 'native', { timeout: 30_000 });

  await r.getByRole('button', { name: 'More actions for native-fixture' }).click();
  await page.getByRole('menuitem', { name: 'Run through Otto’s gateway…' }).click();
  const confirm = page.getByRole('dialog');
  await expect(confirm).toContainText('mcp__native-fixture__<tool>');
  await expect(confirm).toContainText('skills and prompts that name the native tools must be updated');
  await confirm.getByRole('button', { name: 'Switch to governed', exact: true }).click();
  await expect(r).toHaveAttribute('data-runs', 'governed');
  await expect(r.getByText('Not governed', { exact: true })).toHaveCount(0);

  // The daemon agrees: one row, now managed (governed).
  const { ctx } = await apiCtx();
  const list = await (await ctx.get(`${base}/api/v1/workspaces/${workspace}/mcp/servers`)).json();
  const mine = (list as { id: string; managed: boolean }[]).filter((s) => s.id === nativeId);
  expect(mine).toHaveLength(1);
  expect(mine[0].managed).toBe(true);

  await r.getByRole('button', { name: 'More actions for native-fixture' }).click();
  await page.getByRole('menuitem', { name: 'Run natively…' }).click();
  await expect(page.getByRole('dialog')).toContainText('No policy, approval or audit applies');
  await page.getByRole('dialog').getByRole('button', { name: 'Switch to native', exact: true }).click();
  await expect(r).toHaveAttribute('data-runs', 'native');
  const after = await (await ctx.get(`${base}/api/v1/workspaces/${workspace}/mcp/servers`)).json();
  expect((after as { id: string; managed: boolean }[]).find((s) => s.id === nativeId)?.managed).toBe(false);
  await ctx.dispose();
});

test('the merged list fits a phone in light and dark', async ({ page }) => {
  await page.setViewportSize({ width: 390, height: 844 });
  for (const scheme of ['light', 'dark'] as const) {
    await page.emulateMedia({ colorScheme: scheme });
    await page.goto('/#/mcp/servers');
    const r = row(page, 'native-fixture');
    await expect(r.getByText('Not governed', { exact: true })).toBeVisible({ timeout: 30_000 });
    await expect(r.getByRole('button', { name: 'Edit', exact: true })).toBeVisible();
    await expectNoHorizontalOverflow(page);
    await page.screenshot({ path: `/tmp/otto-mcp-one-home-phone-${scheme}.png` });
  }
});
