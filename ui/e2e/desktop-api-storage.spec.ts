import { expect, test } from '@playwright/test';
import { apiCtx, seedWorkspace } from './seed';
import { openPage } from './helpers';
import { isDesktopProject } from './perf';

// perf2 N2: the History list shows what history + automation runs occupy and,
// past a size with no limit set, offers one-click retention presets. Opt-in:
// nothing is deleted until a preset is picked AND confirmed.

let workspaceId = '';

test.beforeAll(async () => {
  const { ctx, base } = await apiCtx();
  try {
    workspaceId = await seedWorkspace(ctx, base);
  } finally {
    await ctx.dispose().catch(() => {});
  }
});

test.beforeEach(async ({ page }, testInfo) => {
  test.skip(!isDesktopProject(testInfo.project.name), 'desktop projects only');
  await page.addInitScript((wsId) => {
    localStorage.setItem('otto_workspace', wsId as string);
    localStorage.setItem('otto_rail_expanded', '0');
    localStorage.setItem('otto_api_side', 'history');
  }, workspaceId);
});

const entry = (wid: string) => ({ id: 'h1', workspace_id: wid, method: 'GET', url: 'https://api.example.com/v1/x', status: 200, duration_ms: 3, executed_at: '2026-09-01T00:00:00Z', request_id: null, source: { kind: 'human', session_id: null, via: null } });

const big = { history_rows: 12_345, history_bytes: 80 * 1024 * 1024, run_rows: 40, step_rows: 900, run_bytes: 2 * 1024 * 1024, max_runs_per_automation: 40 };

test('a big history offers retention presets; nothing is pruned until confirmed', async ({ page }) => {
  const base = `/api/v1/workspaces/${workspaceId}/api-client`;
  let prunes = 0;
  await page.route(`**${base}/history/summaries**`, (r) => r.fulfill({ json: [entry(workspaceId)] }));
  await page.route(`**${base}/storage`, (r) => r.fulfill({ json: big }));
  await page.route(`**${base}/storage/prune`, (r) => {
    prunes++;
    return r.fulfill({ json: { ...big, history_rows: 1_000, history_bytes: 6 * 1024 * 1024 } });
  });
  await openPage(page, 'api');
  const banner = page.locator('.size-banner');
  await expect(banner).toContainText('History is using');
  await expect(banner).toContainText('12,345 requests');
  await expect(page.locator('.ret')).toContainText('80.0 MB');

  // Cancel the confirmation: no setting written, nothing pruned.
  await banner.getByRole('button', { name: 'Keep 1,000' }).click();
  await page.getByRole('button', { name: 'Cancel' }).click();
  expect(prunes).toBe(0);
  await expect(banner).toBeVisible();

  await banner.getByRole('button', { name: 'Keep 1,000' }).click();
  await page.getByRole('button', { name: 'Apply retention' }).click();
  await expect.poll(() => prunes).toBe(1);
  await expect(banner).toHaveCount(0);
  await expect(page.locator('.ret')).toContainText('Keeping the newest 1000');
  await expect(page.locator('.ret')).toContainText('6.0 MB');
});

test('dismissing the storage notice keeps it hidden for the workspace', async ({ page }) => {
  // Each test starts from "no limit" (the other test applies a preset).
  const { ctx, base: daemon } = await apiCtx();
  try {
    const r = await ctx.patch(`${daemon}/api/v1/workspaces/${workspaceId}`, { data: { settings: { api_client: { history_max_rows: 0, history_max_days: 0 } } } });
    expect(r.ok()).toBe(true);
  } finally {
    await ctx.dispose().catch(() => {});
  }
  const base = `/api/v1/workspaces/${workspaceId}/api-client`;
  await page.route(`**${base}/history/summaries**`, (r) => r.fulfill({ json: [entry(workspaceId)] }));
  await page.route(`**${base}/storage`, (r) => r.fulfill({ json: big }));
  await openPage(page, 'api');
  await page.getByRole('button', { name: 'Dismiss storage notice' }).click();
  await expect(page.locator('.size-banner')).toHaveCount(0);
  await page.reload();
  await expect(page.locator('.ret')).toContainText('80.0 MB');
  await expect(page.locator('.size-banner')).toHaveCount(0);
});
