import { test, expect, type APIRequestContext } from '@playwright/test';
import { apiCtx, seedWorkspace } from './seed';

// Desktop coverage for the simplified MCP Control Plane shell: Otto is the
// landing view, the remaining capabilities live on stable sub-routes, and the
// session switch updates one workspace without flattening the settings map.

let base = '';
let workspaceA = '';
let workspaceB = '';

test.describe.configure({ mode: 'serial' });

test.beforeAll(async () => {
  const seeded = await apiCtx();
  base = seeded.base;
  workspaceA = await seedWorkspace(seeded.ctx, base);
  workspaceB = await seedWorkspace(seeded.ctx, base);
  await seeded.ctx.dispose();
});

test.beforeEach(async ({ page }) => {
  await page.addInitScript((wsId) => {
    // addInitScript runs after every navigation; seed the selection once and
    // let subsequent sub-route navigations preserve the browser's own state.
    if (!sessionStorage.getItem('mcp-cp-workspace-seeded')) {
      localStorage.setItem('otto_workspace', wsId as string);
      localStorage.setItem('otto_rail_expanded', '0');
      sessionStorage.setItem('mcp-cp-workspace-seeded', '1');
    }
  }, workspaceA);
});

async function attached(ctx: APIRequestContext, workspaceId: string): Promise<boolean> {
  const response = await ctx.get(`${base}/api/v1/workspaces/${workspaceId}/mcp/session-attach`);
  expect(
    response.ok(),
    `session attach GET → ${response.status()} ${await response.text()}`,
  ).toBeTruthy();
  return ((await response.json()) as { attached: boolean }).attached;
}

test('landing is the Otto server view', async ({ page }) => {
  await page.goto('/#/mcp');

  await expect(page.locator('.otto')).toBeVisible({ timeout: 30_000 });
  await expect(page.locator('[data-testid="mcp-nav-otto"]')).toHaveClass(/\bon\b/);
  await expect(page.locator('input[data-testid="mcp-session-attach"]')).toBeVisible();
  await expect(page.locator('[data-testid="mcp-sessions-panel"]')).toBeVisible();
  await expect(page.locator('.otto .grp-name', { hasText: 'Workflows' })).toBeVisible();

  await expect(page.locator('[data-testid="mcp-expose-panel"]')).toHaveCount(0);
  const expose = page.locator('[data-testid="mcp-expose-toggle"]');
  await expect(expose).toContainText('Connect an external client');
  await expect(expose).toHaveAttribute('aria-expanded', 'false');
  await expose.click();
  await expect(page.locator('[data-testid="mcp-expose-panel"]')).toBeVisible();
  await expect(page.locator('[data-testid="mcp-http-url"]')).toContainText('/api/v1/mcp/http');
});

test('sub-routes expose external servers and activity', async ({ page }) => {
  await page.goto('/#/mcp/servers');
  await expect(page.locator('[data-testid="mcp-add-server"]')).toBeVisible({ timeout: 30_000 });

  await page.goto('/#/mcp/activity');
  await expect(page.locator('[data-testid="mcp-approvals"]')).toBeVisible({ timeout: 30_000 });
  await expect(page.locator('[data-testid="mcp-audit"]')).toBeVisible();
  await page.locator('[data-testid="mcp-audit-bytool"]').click();
  await expect(page.locator('[data-testid="mcp-stats"]')).toBeVisible();
});

test('session attach writes the per-workspace map', async ({ page }) => {
  const { ctx } = await apiCtx();
  try {
    const seed = await ctx.put(`${base}/api/v1/settings`, {
      data: { otto_mcp_enabled: { [workspaceB]: false } },
    });
    expect(seed.ok(), `seed settings → ${seed.status()} ${await seed.text()}`).toBeTruthy();

    await page.goto('/#/mcp');
    const toggle = page.locator('input[data-testid="mcp-session-attach"]');
    await expect(toggle).toBeChecked({ timeout: 30_000 });
    await toggle.uncheck();

    await expect.poll(() => attached(ctx, workspaceA)).toBe(false);
    expect(await attached(ctx, workspaceB), 'workspace B remains detached').toBe(false);

    const settingsResponse = await ctx.get(`${base}/api/v1/settings`);
    expect(
      settingsResponse.ok(),
      `settings GET → ${settingsResponse.status()} ${await settingsResponse.text()}`,
    ).toBeTruthy();
    const settings = (await settingsResponse.json()) as { otto_mcp_enabled?: unknown };
    expect(settings.otto_mcp_enabled).not.toBeNull();
    expect(typeof settings.otto_mcp_enabled).toBe('object');
    expect(Array.isArray(settings.otto_mcp_enabled)).toBe(false);

    await toggle.check();
    await expect.poll(() => attached(ctx, workspaceA)).toBe(true);
    expect(await attached(ctx, workspaceB), 'workspace B stays detached').toBe(false);
  } finally {
    const restore = await ctx.put(`${base}/api/v1/settings`, {
      data: { otto_mcp_enabled: true },
    });
    expect(restore.ok(), `restore settings → ${restore.status()} ${await restore.text()}`).toBeTruthy();
    await ctx.dispose();
  }
});
