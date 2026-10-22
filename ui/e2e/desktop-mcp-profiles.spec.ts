import { test, expect, type APIRequestContext } from '@playwright/test';
import { apiCtx, seedWorkspace } from './seed';

// MCP → Profiles: the built-in profiles (Full is the locked default), creating
// a profile from tool groups, making it the default, its per-profile outside
// connection config, and deleting it (the default falls back to Full).

let base = '';
let workspaceId = '';

test.describe.configure({ mode: 'serial' });

interface Profile { id: string; name: string; is_default: boolean; builtin: boolean }

async function profiles(ctx: APIRequestContext): Promise<Profile[]> {
  const r = await ctx.get(`${base}/api/v1/mcp/profiles`);
  expect(r.ok(), `GET profiles → ${r.status()} ${await r.text()}`).toBeTruthy();
  return ((await r.json()) as { profiles: Profile[] }).profiles;
}

async function cleanup(ctx: APIRequestContext): Promise<void> {
  await ctx.post(`${base}/api/v1/mcp/profiles/full/default`);
  for (const p of await profiles(ctx)) if (!p.builtin) await ctx.delete(`${base}/api/v1/mcp/profiles/${p.id}`);
}

test.beforeAll(async () => {
  const seeded = await apiCtx();
  base = seeded.base;
  workspaceId = await seedWorkspace(seeded.ctx, base);
  await cleanup(seeded.ctx);
  await seeded.ctx.dispose();
});

test.afterAll(async () => {
  const { ctx } = await apiCtx();
  await cleanup(ctx);
  await ctx.dispose();
});

test.beforeEach(async ({ page }) => {
  await page.addInitScript((wsId) => {
    if (!sessionStorage.getItem('mcp-prof-seeded')) {
      localStorage.setItem('otto_workspace', wsId as string);
      sessionStorage.setItem('mcp-prof-seeded', '1');
    }
  }, workspaceId);
});

test('built-ins list with Full as the locked default and a profile config', async ({ page }) => {
  await page.goto('/#/mcp/profiles');
  const root = page.locator('[data-testid="mcp-profiles"]');
  await expect(root).toBeVisible({ timeout: 30_000 });
  for (const id of ['full', 'database', 'issues', 'git', 'cloud']) {
    await expect(root.locator(`[data-testid="mcp-profile-${id}"]`)).toBeVisible();
  }
  // Opens on the default (Full): the plain all-tools connection.
  await expect(root.locator('[data-testid="mcp-profile-config"]')).toContainText('"mcp-server"');
  await root.locator('[data-testid="mcp-profile-database"]').click();
  await expect(root.locator('[data-testid="mcp-profile-config"]')).toContainText('--profile');
  await root.getByRole('tab', { name: 'HTTP' }).click();
  await expect(root.locator('[data-testid="mcp-profile-config"]')).toContainText('/api/v1/mcp/http?profile=database');
});

test('create a profile from groups, make it the default, then delete it', async ({ page }) => {
  await page.goto('/#/mcp/profiles');
  await page.locator('[data-testid="mcp-profile-new"]').click();
  const dialog = page.getByRole('dialog');
  await dialog.getByLabel('Name').fill('Data + Docs');
  await dialog.locator('[data-testid="mcp-profile-group-database"]').check();
  await dialog.locator('[data-testid="mcp-profile-group-vault"]').check();
  await dialog.locator('[data-testid="mcp-profile-save"]').click();
  await expect(dialog).toBeHidden();

  const { ctx } = await apiCtx();
  const created = (await profiles(ctx)).find((p) => p.name === 'Data + Docs');
  await ctx.dispose();
  expect(created).toBeTruthy();
  const row = page.locator(`[data-testid="mcp-profile-${created!.id}"]`);
  await expect(row).toBeVisible();
  await row.click();
  await page.locator('[data-testid="mcp-profile-make-default"]').click();
  await expect(row).toContainText('Default');

  await page.getByRole('button', { name: /More actions for Data \+ Docs/ }).click();
  await page.getByRole('menuitem', { name: 'Delete…' }).click();
  await page.getByRole('dialog').getByRole('button', { name: 'Delete', exact: true }).click();
  await expect(row).toBeHidden();
  await expect(page.locator('[data-testid="mcp-profile-full"]')).toContainText('Default');
});
