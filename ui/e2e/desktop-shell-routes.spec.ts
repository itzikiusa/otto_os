import { test, expect, type APIRequestContext, type Page } from '@playwright/test';
import { apiCtx, seedWorkspace } from './seed';

// ─────────────────────────────────────────────────────────────────────────────
// Shell routing, sidebar order and workspace handling (app-review wave 1):
// - an unknown route shows "Page not found" (not a silent Agents page) and is
//   never saved as a resume point / last route;
// - a module missing from the saved sidebar order (gated off when it was
//   saved) comes back next to its registry neighbour, not at the bottom;
// - the workspace switcher lists the recently used workspaces first;
// - removing a workspace that still has sessions asks what to do with them,
//   and "Removed workspaces…" restores it.
// ─────────────────────────────────────────────────────────────────────────────

let ctx: APIRequestContext;
let base = '';
let run = '';

test.beforeEach(async ({}, info) => {
  test.skip(info.project.name !== 'desktop-browser', 'desktop-browser project only');
  const c = await apiCtx();
  ctx = c.ctx;
  base = c.base;
  run = Math.random().toString(36).slice(2, 7);
});

test.afterEach(async () => {
  await ctx?.dispose();
});

async function boot(page: Page, wsId: string, extra: Record<string, string> = {}, hash = '#/agents'): Promise<void> {
  await page.addInitScript(
    ({ wsId, extra }) => {
      // Only on the first load: a reload must see what the app itself stored.
      if (sessionStorage.getItem('e2e_seeded')) return;
      sessionStorage.setItem('e2e_seeded', '1');
      localStorage.setItem('otto_workspace', wsId);
      for (const [k, v] of Object.entries(extra)) localStorage.setItem(k, v);
    },
    { wsId, extra },
  );
  await page.goto(`/${hash}`);
}

const workspacesSection = (page: Page) =>
  page.locator('.nav-section', { has: page.locator('.nav-label', { hasText: 'Workspaces' }) });

test('an unknown route shows "Page not found" with a way back, and is never remembered', async ({ page }) => {
  const ws = await seedWorkspace(ctx, base, `Routes ${run}`);
  await boot(page, ws, {}, '#/nope-not-a-page/42');
  await expect(page.getByRole('heading', { name: 'Page not found' }).first()).toBeVisible({ timeout: 20_000 });
  await expect(page.getByText('#/nope-not-a-page/42')).toBeVisible();
  // Nothing in the sidebar claims it.
  await expect(page.locator('.nav-item.active[data-nav-id]')).toHaveCount(0);
  const stored = await page.evaluate(() => JSON.stringify({ ...localStorage }));
  expect(stored).not.toContain('nope-not-a-page');
  await page.getByRole('button', { name: 'Go to Agents' }).click();
  await expect(page).toHaveURL(/#\/agents$/);
  // A fresh load without a hash lands on the default page, not the bad route.
  await page.goto('/');
  await expect(page.locator('[data-nav-id="agents"]').first()).toBeVisible({ timeout: 20_000 });
  await expect(page.getByRole('heading', { name: 'Page not found' })).toHaveCount(0);
});

test('a module missing from the saved order comes back beside its neighbour', async ({ page }) => {
  const ws = await seedWorkspace(ctx, base, `Order ${run}`);
  // Saved while Kubernetes was gated off, with API moved above AWS.
  const infra = ['connections', 'api', 'aws', 'browser', 'mcp'];
  await boot(page, ws, {
    otto_sidebar_order: JSON.stringify(['home', 'agents', 'threads', 'rooms', ...infra]),
    otto_sidebar_known_modules: JSON.stringify(['home', 'agents', 'threads', 'rooms', ...infra]),
    otto_rail_expanded: '1',
  });
  await expect(page.locator('[data-nav-id="kubernetes"]').first()).toBeVisible({ timeout: 20_000 });
  const order = await page.$$eval('.nav-item[data-nav-id]', (els) => els.map((e) => e.getAttribute('data-nav-id')));
  const shown = order.filter((id): id is string => !!id && ['connections', 'api', 'aws', 'kubernetes', 'browser', 'mcp'].includes(id));
  // Kubernetes follows AWS (its registry predecessor) wherever AWS sits.
  expect(shown).toEqual(['connections', 'api', 'aws', 'kubernetes', 'browser', 'mcp']);
  // Once it has been available it is known, and the saved order keeps its slot.
  await expect
    .poll(() => page.evaluate(() => JSON.parse(localStorage.getItem('otto_sidebar_known_modules') ?? '[]').includes('kubernetes')))
    .toBe(true);
});

test('the workspace switcher lists recently used workspaces first', async ({ page }) => {
  const ids: string[] = [];
  for (let i = 0; i < 6; i++) ids.push(await seedWorkspace(ctx, base, `Recent ${run} ${i}`));
  await boot(page, ids[0], { otto_ws_recent: JSON.stringify([ids[4], ids[2]]), otto_rail_expanded: '1' });
  const chip = page.getByTestId('agents-current-ws');
  await expect(chip).toBeVisible({ timeout: 20_000 });
  await chip.click();
  const labels = await page.locator('.ctx-item').allInnerTexts();
  const ours = labels.map((l) => l.trim()).filter((l) => l.startsWith(`Recent ${run}`));
  // The current workspace was just used too, so it leads; then the stored recents.
  expect(ours.slice(0, 3)).toEqual([`Recent ${run} 0`, `Recent ${run} 4`, `Recent ${run} 2`]);
  expect(new Set(ours).size).toBe(ours.length);
});

test('removing a workspace with sessions asks about them; "Removed workspaces" restores it', async ({ page }) => {
  const home = await seedWorkspace(ctx, base, `Home ${run}`);
  const doomed = await seedWorkspace(ctx, base, `Doomed ${run}`);
  const r = await ctx.post(`${base}/api/v1/workspaces/${doomed}/sessions`, {
    data: { kind: 'agent', provider: 'shell', title: `Left ${run}`, cwd: '/tmp', meta: { origin: 'e2e' } },
  });
  expect(r.ok()).toBeTruthy();
  const sessionId = (await r.json()).id as string;
  await boot(page, home, { otto_rail_expanded: '1' });
  const row = workspacesSection(page).locator('.nav-item', { hasText: `Doomed ${run}` });
  await expect(row).toBeVisible({ timeout: 20_000 });
  await row.click({ button: 'right' });
  await page.locator('.ctx-item', { hasText: 'Remove workspace…' }).click();
  const dialog = page.getByRole('dialog');
  await expect(dialog).toContainText('still has 1 session');
  await dialog.getByRole('button', { name: 'Stop and archive its sessions' }).click();
  await expect(row).toHaveCount(0, { timeout: 10_000 });
  await expect
    .poll(async () => ((await (await ctx.get(`${base}/api/v1/sessions/${sessionId}`)).json()) as { archived: boolean }).archived, { timeout: 15_000 })
    .toBe(true);

  // Restore from the switcher's "Removed workspaces…".
  await page.getByTestId('agents-current-ws').click();
  await page.locator('.ctx-item', { hasText: 'Removed workspaces…' }).click();
  const list = page.getByTestId('removed-workspaces');
  await expect(list).toContainText(`Doomed ${run}`);
  await list.locator('li', { hasText: `Doomed ${run}` }).getByRole('button', { name: 'Restore' }).click();
  await expect(list.locator('li', { hasText: `Doomed ${run}` })).toHaveCount(0);
  await page.keyboard.press('Escape');
  await expect(workspacesSection(page).locator('.nav-item', { hasText: `Doomed ${run}` })).toBeVisible();
});
